// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"strings"

	"golang.org/x/crypto/bcrypt"
)

// Preserve the old verifier, never convert a bcrypt digest into PBKDF2 or make
// an imported protected order unprotected. Shared dependency pin belongs to root.
func validLegacyPickupCode(p *pickupProtection, code string) bool {
	if p == nil || p.Salt != "" || p.Hash != "" || len(code) < 8 || len(code) > 72 {
		return false
	}
	cost, err := bcrypt.Cost([]byte(p.LegacyBcrypt))
	if err != nil || cost < 4 || cost > 14 {
		return false
	}
	return bcrypt.CompareHashAndPassword([]byte(p.LegacyBcrypt), []byte(code)) == nil
}

type legacyPickupAliasIndex interface {
	LegacyPickupToken(context.Context, string) (string, error)
}

func (s *Service) canonicalPickupToken(ctx context.Context, token string) (string, error) {
	if strings.Contains(token, ".") {
		return token, nil
	}
	// The pinned old implementation minted 32 random bytes as raw base64url.
	if len(token) != 43 {
		return "", ErrNotFound
	}
	raw, err := base64.RawURLEncoding.DecodeString(token)
	if err != nil || len(raw) != 32 {
		return "", ErrNotFound
	}
	index, ok := s.repo.(legacyPickupAliasIndex)
	if !ok || Missing(index) {
		return "", ErrNotFound
	}
	hash := sha256.Sum256([]byte(token))
	// Only resolve an imported alias; the ordinary Pickup method still checks
	// its exact stored token, buyer session, original password and order state.
	return index.LegacyPickupToken(ctx, hex.EncodeToString(hash[:]))
}
