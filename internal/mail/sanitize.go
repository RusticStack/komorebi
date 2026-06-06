package mail

import (
	"html"
	"net/url"
	"strings"

	xhtml "golang.org/x/net/html"
)

type sanitizedHTML struct {
	HTML                 string
	Text                 string
	RemoteContentBlocked int
	UnsafeRemoved        bool
}

type sanitizeOptions struct {
	AllowRemoteContent bool
}

var allowedHTMLTags = map[string]bool{
	"a":          true,
	"b":          true,
	"blockquote": true,
	"br":         true,
	"code":       true,
	"div":        true,
	"em":         true,
	"i":          true,
	"li":         true,
	"ol":         true,
	"p":          true,
	"pre":        true,
	"span":       true,
	"strong":     true,
	"u":          true,
	"ul":         true,
}

var voidHTMLTags = map[string]bool{
	"br":  true,
	"img": true,
}

var remoteContentTags = map[string]bool{
	"audio":  true,
	"embed":  true,
	"iframe": true,
	"img":    true,
	"link":   true,
	"object": true,
	"source": true,
	"track":  true,
	"video":  true,
}

func sanitizeMessageHTML(input string) sanitizedHTML {
	return sanitizeMessageHTMLWithOptions(input, sanitizeOptions{})
}

func sanitizeMessageHTMLWithOptions(input string, options sanitizeOptions) sanitizedHTML {
	if strings.TrimSpace(input) == "" {
		return sanitizedHTML{}
	}

	tokenizer := xhtml.NewTokenizer(strings.NewReader(input))
	var out strings.Builder
	var text strings.Builder
	var openTags []string
	skipDepth := 0
	result := sanitizedHTML{}

	for {
		tokenType := tokenizer.Next()
		if tokenType == xhtml.ErrorToken {
			break
		}

		token := tokenizer.Token()
		tag := strings.ToLower(token.Data)

		if skipDepth > 0 {
			if tokenType == xhtml.StartTagToken && (tag == "script" || tag == "style") {
				skipDepth++
			}
			if tokenType == xhtml.EndTagToken && (tag == "script" || tag == "style") {
				skipDepth--
			}
			result.UnsafeRemoved = true
			continue
		}

		switch tokenType {
		case xhtml.TextToken:
			textValue := token.Data
			out.WriteString(html.EscapeString(textValue))
			text.WriteString(textValue)
		case xhtml.StartTagToken, xhtml.SelfClosingTagToken:
			if tag == "script" || tag == "style" {
				skipDepth = 1
				result.UnsafeRemoved = true
				continue
			}
			if tag == "img" && options.AllowRemoteContent {
				src, alt, ok := safeImageAttrs(token.Attr)
				if !ok {
					result.RemoteContentBlocked++
					result.UnsafeRemoved = true
					continue
				}
				out.WriteString(`<img src="`)
				out.WriteString(html.EscapeString(src))
				out.WriteString(`" alt="`)
				out.WriteString(html.EscapeString(alt))
				out.WriteString(`">`)
				continue
			}
			if remoteContentTags[tag] {
				result.RemoteContentBlocked++
				result.UnsafeRemoved = true
				continue
			}
			if !allowedHTMLTags[tag] {
				result.UnsafeRemoved = true
				continue
			}
			out.WriteByte('<')
			out.WriteString(tag)
			if tag == "a" {
				for _, attr := range token.Attr {
					if strings.EqualFold(attr.Key, "href") && safeHref(attr.Val) {
						out.WriteString(` href="`)
						out.WriteString(html.EscapeString(attr.Val))
						out.WriteString(`" rel="noreferrer noopener"`)
						break
					}
				}
			}
			if voidHTMLTags[tag] || tokenType == xhtml.SelfClosingTagToken {
				out.WriteString(">")
				if tag == "br" {
					text.WriteByte('\n')
				}
				continue
			}
			out.WriteString(">")
			openTags = append(openTags, tag)
		case xhtml.EndTagToken:
			if !allowedHTMLTags[tag] || voidHTMLTags[tag] {
				continue
			}
			idx := lastOpenTag(openTags, tag)
			if idx == -1 {
				continue
			}
			for i := len(openTags) - 1; i >= idx; i-- {
				out.WriteString("</")
				out.WriteString(openTags[i])
				out.WriteByte('>')
			}
			openTags = openTags[:idx]
		}
	}

	for i := len(openTags) - 1; i >= 0; i-- {
		out.WriteString("</")
		out.WriteString(openTags[i])
		out.WriteByte('>')
	}

	result.HTML = strings.TrimSpace(out.String())
	result.Text = strings.TrimSpace(text.String())
	return result
}

func safeImageAttrs(attrs []xhtml.Attribute) (string, string, bool) {
	var src, alt string
	for _, attr := range attrs {
		switch strings.ToLower(attr.Key) {
		case "src":
			if safeRemoteURL(attr.Val) {
				src = strings.TrimSpace(attr.Val)
			}
		case "alt":
			alt = strings.TrimSpace(attr.Val)
			if len(alt) > 200 {
				alt = alt[:200]
			}
		}
	}
	return src, alt, src != ""
}

func safeHref(value string) bool {
	value = strings.TrimSpace(value)
	if value == "" || strings.ContainsAny(value, "\r\n\t") {
		return false
	}
	parsed, err := url.Parse(value)
	if err != nil || parsed.Scheme == "" {
		return false
	}
	switch strings.ToLower(parsed.Scheme) {
	case "https", "http", "mailto":
		return true
	default:
		return false
	}
}

func safeRemoteURL(value string) bool {
	value = strings.TrimSpace(value)
	if value == "" || strings.ContainsAny(value, "\r\n\t") {
		return false
	}
	parsed, err := url.Parse(value)
	if err != nil || parsed.Scheme == "" || parsed.Host == "" {
		return false
	}
	switch strings.ToLower(parsed.Scheme) {
	case "https", "http":
		return true
	default:
		return false
	}
}

func lastOpenTag(tags []string, tag string) int {
	for i := len(tags) - 1; i >= 0; i-- {
		if tags[i] == tag {
			return i
		}
	}
	return -1
}

func textToSafeHTML(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	escaped := html.EscapeString(value)
	escaped = strings.ReplaceAll(escaped, "\r\n", "\n")
	escaped = strings.ReplaceAll(escaped, "\r", "\n")
	parts := strings.Split(escaped, "\n")
	for i, part := range parts {
		parts[i] = "<p>" + part + "</p>"
	}
	return strings.Join(parts, "")
}
