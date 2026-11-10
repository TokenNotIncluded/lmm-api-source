// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"io"
	"strings"
)

// Purpose is authenticated, so settings and operation snapshots cannot be
// decrypted as another provider's secret. The v1 envelope preserves APP01.
func sealCredentialPurpose(aead cipher.AEAD, purpose string, plain []byte) (string, error) {
	if aead == nil {
		return "", ErrUnavailable
	}
	nonce := make([]byte, aead.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return "", ErrUnavailable
	}
	return "v1:" + base64.RawURLEncoding.EncodeToString(aead.Seal(nonce, nonce, plain, []byte(purpose))), nil
}

func openCredentialPurpose(aead cipher.AEAD, purpose, sealed string) ([]byte, error) {
	if aead == nil || !strings.HasPrefix(sealed, "v1:") {
		return nil, ErrUnavailable
	}
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(sealed, "v1:"))
	if err != nil || len(raw) < aead.NonceSize() {
		return nil, ErrUnavailable
	}
	plain, err := aead.Open(nil, raw[:aead.NonceSize()], raw[aead.NonceSize():], []byte(purpose))
	if err != nil {
		return nil, ErrUnavailable
	}
	return plain, nil
}
