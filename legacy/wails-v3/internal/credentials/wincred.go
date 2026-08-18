package credentials

import (
	"encoding/json"
	"fmt"
	"time"

	"github.com/danieljoos/wincred"
)

// Windows Credential Manager backend (best practices 2026).
// Stores structured data (prefer refresh tokens over passwords).
// Target names are descriptive. Memory zeroing recommended after use in production.

const targetPrefix = "Komorebi/"

// CredentialData is the structured blob we store (supports OAuth refresh + expiry).
type CredentialData struct {
	Secret        string    `json:"secret"` // legacy password field for generic providers
	Expiry        time.Time `json:"expiry,omitempty"`
	Username      string    `json:"username,omitempty"`
	AuthMode      string    `json:"authMode,omitempty"`
	ProviderKind  string    `json:"providerKind,omitempty"`
	AccessToken   string    `json:"accessToken,omitempty"`
	RefreshToken  string    `json:"refreshToken,omitempty"`
	TokenEndpoint string    `json:"tokenEndpoint,omitempty"`
	ClientID      string    `json:"clientId,omitempty"`
}

// Save stores credential data securely.
func Save(accountID string, data CredentialData) error {
	target := targetPrefix + accountID

	blob, err := json.Marshal(data)
	if err != nil {
		return err
	}

	cred := wincred.NewGenericCredential(target)
	cred.CredentialBlob = blob
	cred.Persist = wincred.PersistLocalMachine
	if err := cred.Write(); err != nil {
		return fmt.Errorf("wincred write: %w", err)
	}
	return nil
}

// Get retrieves and unmarshals the credential.
func Get(accountID string) (CredentialData, error) {
	cred, err := wincred.GetGenericCredential(targetPrefix + accountID)
	if err != nil {
		return CredentialData{}, fmt.Errorf("wincred get: %w", err)
	}

	var data CredentialData
	if err := json.Unmarshal(cred.CredentialBlob, &data); err != nil {
		// Fallback for legacy plain-string storage during transition
		data = CredentialData{Secret: string(cred.CredentialBlob)}
	}
	return data, nil
}

// Delete removes the credential.
func Delete(accountID string) error {
	cred, err := wincred.GetGenericCredential(targetPrefix + accountID)
	if err == wincred.ErrElementNotFound {
		return nil
	}
	if err != nil {
		return fmt.Errorf("wincred get for delete: %w", err)
	}
	err = cred.Delete()
	if err != nil {
		return fmt.Errorf("wincred delete: %w", err)
	}
	return nil
}
