package mail

import (
	"encoding/json"
	"fmt"
	"net"
	netmail "net/mail"
	"net/url"
	"path/filepath"
	"regexp"
	"strings"
	"unicode"
)

const (
	maxIDLength         = 160
	maxSubjectLength    = 300
	maxDisplayName      = 120
	maxHTMLBodyLength   = 512 * 1024
	maxSearchQuery      = 200
	maxAttachmentSize   = 25 * 1024 * 1024
	maxAttachmentName   = 180
	defaultMessageLimit = 60
	maxMessageLimit     = 120
)

var (
	idPattern   = regexp.MustCompile(`^[A-Za-z0-9._:@+-]+$`)
	hostPattern = regexp.MustCompile(`^[A-Za-z0-9.-]+$`)
	wordPattern = regexp.MustCompile(`[\pL\pN_@.+-]+`)
)

func validateID(label, value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return "", fmt.Errorf("%s is required", label)
	}
	if len(value) > maxIDLength || !idPattern.MatchString(value) {
		return "", fmt.Errorf("%s is invalid", label)
	}
	return value, nil
}

func validateLimit(limit, fallback, max int) int {
	if limit <= 0 {
		return fallback
	}
	if limit > max {
		return max
	}
	return limit
}

func validateOffset(offset int64) (int64, error) {
	if offset < 0 || offset > 1_000_000 {
		return 0, fmt.Errorf("cursor is invalid")
	}
	return offset, nil
}

func validateEmail(value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" || len(value) > 254 {
		return "", fmt.Errorf("email is invalid")
	}
	addr, err := netmail.ParseAddress(value)
	if err != nil || addr.Address == "" || strings.ContainsAny(addr.Address, "\r\n") {
		return "", fmt.Errorf("email is invalid")
	}
	return strings.ToLower(addr.Address), nil
}

func validateAddressList(values []string, allowEmpty bool) ([]string, error) {
	var result []string
	seen := map[string]bool{}
	for _, value := range values {
		email, err := validateEmail(value)
		if err != nil {
			return nil, err
		}
		if !seen[email] {
			result = append(result, email)
			seen[email] = true
		}
	}
	if len(result) == 0 && !allowEmpty {
		return nil, fmt.Errorf("at least one recipient is required")
	}
	return result, nil
}

func normalizeAddressJSON(value string, allowEmpty bool) (string, []string, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		if allowEmpty {
			return "[]", nil, nil
		}
		return "", nil, fmt.Errorf("recipient is required")
	}

	var raw []string
	if strings.HasPrefix(value, "[") {
		if err := json.Unmarshal([]byte(value), &raw); err != nil {
			return "", nil, fmt.Errorf("recipient list is invalid")
		}
	} else {
		raw = strings.FieldsFunc(value, func(r rune) bool {
			return r == ',' || r == ';'
		})
	}

	addresses, err := validateAddressList(raw, allowEmpty)
	if err != nil {
		return "", nil, err
	}
	encoded, err := json.Marshal(addresses)
	if err != nil {
		return "", nil, err
	}
	return string(encoded), addresses, nil
}

func validateHost(value string) (string, error) {
	host := strings.ToLower(strings.TrimSpace(value))
	if host == "" || len(host) > 253 {
		return "", fmt.Errorf("host is invalid")
	}
	if strings.Contains(host, "://") || strings.ContainsAny(host, "/\\?#@ \t\r\n") {
		return "", fmt.Errorf("host must be a hostname, not a URL")
	}
	if ip := net.ParseIP(host); ip != nil {
		return "", fmt.Errorf("IP literals are not accepted for mail provider setup")
	}
	if !hostPattern.MatchString(host) || strings.Contains(host, "..") {
		return "", fmt.Errorf("host is invalid")
	}
	labels := strings.Split(host, ".")
	for _, label := range labels {
		if label == "" || len(label) > 63 || strings.HasPrefix(label, "-") || strings.HasSuffix(label, "-") {
			return "", fmt.Errorf("host is invalid")
		}
	}
	return host, nil
}

func validatePort(value int) (int, error) {
	if value < 1 || value > 65535 {
		return 0, fmt.Errorf("port is invalid")
	}
	return value, nil
}

func validateSecurity(value string) (string, error) {
	value = strings.ToLower(strings.TrimSpace(value))
	switch value {
	case "tls", "starttls":
		return value, nil
	default:
		return "", fmt.Errorf("mail security must be tls or starttls")
	}
}

func validateSubject(value string) (string, error) {
	value = strings.TrimSpace(value)
	if len(value) > maxSubjectLength || strings.ContainsAny(value, "\r\n") {
		return "", fmt.Errorf("subject is invalid")
	}
	return value, nil
}

func validateDisplayName(value string) (string, error) {
	value = strings.TrimSpace(value)
	if len(value) > maxDisplayName || strings.ContainsAny(value, "\r\n") {
		return "", fmt.Errorf("display name is invalid")
	}
	return value, nil
}

func validateHTMLBody(value string) (string, error) {
	if len(value) > maxHTMLBodyLength {
		return "", fmt.Errorf("message body is too large")
	}
	return value, nil
}

func validateRedirectURI(value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" || len(value) > 500 {
		return "", fmt.Errorf("redirect URI is invalid")
	}
	parsed, err := url.Parse(value)
	if err != nil || parsed.Scheme == "" {
		return "", fmt.Errorf("redirect URI is invalid")
	}
	switch parsed.Scheme {
	case "http":
		host := strings.ToLower(parsed.Hostname())
		if host != "127.0.0.1" && host != "localhost" {
			return "", fmt.Errorf("http redirect URI must be localhost")
		}
	case "https":
		if parsed.Hostname() == "" {
			return "", fmt.Errorf("redirect URI is invalid")
		}
	default:
		if !strings.HasPrefix(parsed.Scheme, "komorebi") {
			return "", fmt.Errorf("redirect URI scheme is not allowed")
		}
	}
	return value, nil
}

func validateClientID(value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" || len(value) > 300 || strings.ContainsAny(value, "\r\n\t ") {
		return "", fmt.Errorf("OAuth client ID is required")
	}
	return value, nil
}

func validateOAuthCode(value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" || len(value) > 4096 || strings.ContainsAny(value, "\r\n") {
		return "", fmt.Errorf("OAuth authorization code is invalid")
	}
	return value, nil
}

func normalizeSearchQuery(input string) (string, error) {
	input = strings.TrimSpace(input)
	if input == "" {
		return "", nil
	}
	if len(input) > maxSearchQuery {
		return "", fmt.Errorf("search query is too long")
	}
	for _, r := range input {
		if unicode.IsControl(r) && !unicode.IsSpace(r) {
			return "", fmt.Errorf("search query contains control characters")
		}
	}

	terms := wordPattern.FindAllString(input, 8)
	if len(terms) == 0 {
		return "", nil
	}
	quoted := make([]string, 0, len(terms))
	for _, term := range terms {
		term = strings.Trim(term, ".+-_@")
		if term == "" {
			continue
		}
		quoted = append(quoted, `"`+strings.ReplaceAll(term, `"`, `""`)+`"`)
	}
	return strings.Join(quoted, " "), nil
}

func safeAttachmentFilename(value string) (string, error) {
	value = strings.TrimSpace(filepath.Base(value))
	if value == "" || value == "." || value == ".." || len(value) > maxAttachmentName {
		return "", fmt.Errorf("attachment filename is invalid")
	}
	if strings.ContainsAny(value, `/\:*?"<>|`+"\r\n") {
		return "", fmt.Errorf("attachment filename is invalid")
	}
	return value, nil
}

func validateMIME(value string) (string, error) {
	value = strings.ToLower(strings.TrimSpace(value))
	if value == "" {
		return "application/octet-stream", nil
	}
	if len(value) > 120 || strings.ContainsAny(value, "\r\n; ") || !strings.Contains(value, "/") {
		return "", fmt.Errorf("mime type is invalid")
	}
	return value, nil
}
