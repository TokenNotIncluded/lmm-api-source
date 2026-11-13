package store

import (
	"context"
	"encoding/json"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

type extoreConnectionStatus uint8

const (
	extoreConnectionActive extoreConnectionStatus = iota
	extoreConnectionRevoked
	extoreConnectionRefreshPending
	extoreConnectionExpired
)

func (row extoreConnectionRow) connectionStatus(now int64) extoreConnectionStatus {
	switch {
	case row.RevokedAt != 0:
		return extoreConnectionRevoked
	case row.RefreshPending:
		return extoreConnectionRefreshPending
	case row.ExpiresAt <= now:
		return extoreConnectionExpired
	default:
		return extoreConnectionActive
	}
}

func (s *Service) readExtoreConnection(ctx context.Context, userID int64, id string) (extoreConnectionRow, error) {
	var row extoreConnectionRow
	err := s.repo.Within(ctx, extoreSellerShop(userID), func(tx Tx) error {
		return tx.Get(extoreConnectionKind, id, &row)
	})
	return row, err
}

func (s *Service) decryptExtoreCredentials(row extoreConnectionRow) (extore.Credentials, error) {
	secret, err := s.extore.open(extoreConnectionAAD(row.ID), row.Ciphertext)
	if err != nil {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	var credentials extore.Credentials
	if json.Unmarshal(secret, &credentials) != nil {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	return credentials, nil
}

func (s *Service) decryptExtoreSyncCredentials(row extoreConnectionRow, now int64) (extore.Credentials, error) {
	if row.connectionStatus(now) != extoreConnectionActive {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	credentials, err := s.decryptExtoreCredentials(row)
	if err != nil || credentials.Access == "" || credentials.Refresh == "" {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	return credentials, nil
}

func (s *Service) decryptExtoreRevocationCredentials(row extoreConnectionRow, now int64) (extore.Credentials, error) {
	// Retain the reauthorization policy for expired/revoked grants and unresolved
	// refreshes. Revocation itself requires only the refresh token, not access.
	if row.connectionStatus(now) != extoreConnectionActive {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	credentials, err := s.decryptExtoreCredentials(row)
	if err != nil || credentials.Refresh == "" {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	return credentials, nil
}
