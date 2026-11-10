// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"database/sql"
	_ "embed"
	"encoding/base64"
	"errors"
	"io"
	"strings"
	"time"
)

//go:embed script_repository_credentials_schema.sql
var ScriptRepositoryCredentialsSchema string

// WithCredentialKey is host configuration, never a browser input. Key material
// must come from a separate private 32-byte file. No startup DDL is performed.
func (s *ScriptRepository) WithCredentialKey(ctx context.Context, key []byte) error {
	if s == nil || s.db == nil || len(key) != 32 {
		return ErrInvalid
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return ErrInvalid
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var installed bool
	if s.db.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM pg_attribute WHERE attrelid='lmm_operations.script_repository'::regclass AND attname='github_key_ciphertext' AND atttypid='text'::regtype AND attnotnull AND NOT attisdropped)`).Scan(&installed) != nil || !installed {
		return ErrUnavailable
	}
	s.credentials = aead
	return nil
}

const scriptCredentialAAD = "lmm_operations.script_repository.github_key.v1"

func validRepositoryToken(token string) bool {
	if len(token) == 0 || len(token) > 4096 {
		return false
	}
	for _, b := range []byte(token) {
		if b < 33 || b > 126 {
			return false
		}
	}
	return true
}
func (s *ScriptRepository) sealCredential(token string) (string, error) {
	if s.credentials == nil {
		return "", ErrUnavailable
	}
	if !validRepositoryToken(token) {
		return "", ErrInvalid
	}
	nonce := make([]byte, s.credentials.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return "", ErrUnavailable
	}
	return "v1:" + base64.RawURLEncoding.EncodeToString(s.credentials.Seal(nonce, nonce, []byte(token), []byte(scriptCredentialAAD))), nil
}
func (s *ScriptRepository) openCredential(sealed string) (string, error) {
	if s.credentials == nil || !strings.HasPrefix(sealed, "v1:") {
		return "", ErrUnavailable
	}
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(sealed, "v1:"))
	if err != nil || len(raw) < s.credentials.NonceSize() {
		return "", ErrUnavailable
	}
	plain, err := s.credentials.Open(nil, raw[:s.credentials.NonceSize()], raw[s.credentials.NonceSize():], []byte(scriptCredentialAAD))
	if err != nil || !validRepositoryToken(string(plain)) {
		return "", ErrUnavailable
	}
	return string(plain), nil
}
func (s *ScriptRepository) credential(ctx context.Context, revision int64) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var sealed string
	// to_jsonb also supports the explicitly unupgraded public-only schema.
	err := s.db.QueryRowContext(ctx, `SELECT coalesce(to_jsonb(r)->>'github_key_ciphertext','') FROM lmm_operations.script_repository r WHERE singleton AND revision=$1`, revision).Scan(&sealed)
	if errors.Is(err, sql.ErrNoRows) {
		return "", ErrConflict
	}
	if err != nil {
		return "", ErrUnavailable
	}
	if sealed == "" {
		return "", nil
	}
	return s.openCredential(sealed)
}
