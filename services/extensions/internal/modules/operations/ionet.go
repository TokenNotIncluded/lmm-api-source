// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"database/sql"
	_ "embed"
	"errors"
	"net/http"
	"regexp"
	"time"
)

//go:embed ionet_schema.sql
var IoNetSchema string

const ioNetSettingsPurpose = "lmm_operations.ionet.settings.v1"

var ErrIoNetUpstream = errors.New("io.net request unavailable")
var ioNetID = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`)
var ioNetOperationKey = regexp.MustCompile(`^[A-Za-z0-9_-]{16,128}$`)

type IoNet struct {
	db          *sql.DB
	client      *http.Client
	credentials cipher.AEAD
}
type IoNetSettings struct {
	Revision   int64  `json:"revision"`
	Provider   string `json:"provider"`
	Enabled    bool   `json:"enabled"`
	Configured bool   `json:"configured"`
	CanConnect bool   `json:"can_connect"`
}

func NewIoNet(db *sql.DB) *IoNet { return &IoNet{db: db, client: newRSS(nil).client} }

// Configure before serving. Missing encryption material never downgrades a
// configured private provider to an anonymous request.
func (s *IoNet) WithCredentialKey(ctx context.Context, key []byte) error {
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
	if s.db.QueryRowContext(ctx, `SELECT to_regclass('lmm_operations.ionet_config') IS NOT NULL AND to_regclass('lmm_operations.ionet_operations') IS NOT NULL`).Scan(&installed) != nil || !installed {
		return ErrUnavailable
	}
	s.credentials = aead
	return nil
}

func (s *IoNet) settings(ctx context.Context) (IoNetSettings, string, error) {
	if s == nil || s.db == nil {
		return IoNetSettings{}, "", ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	out := IoNetSettings{Provider: "io.net"}
	var sealed string
	if s.db.QueryRowContext(ctx, `SELECT revision,enabled,api_key_ciphertext FROM lmm_operations.ionet_config WHERE singleton`).Scan(&out.Revision, &out.Enabled, &sealed) != nil {
		return IoNetSettings{}, "", ErrUnavailable
	}
	out.Configured = sealed != ""
	out.CanConnect = out.Enabled && out.Configured && s.credentials != nil
	return out, sealed, nil
}
func (s *IoNet) Settings(ctx context.Context) (IoNetSettings, error) {
	out, _, err := s.settings(ctx)
	return out, err
}
func (s *IoNet) SaveSettings(ctx context.Context, actor, revision int64, enabled bool, key string, clear bool) (IoNetSettings, error) {
	if !validID(actor) || revision < 0 || revision >= MaxID || clear && key != "" {
		return IoNetSettings{}, ErrInvalid
	}
	current, sealed, err := s.settings(ctx)
	if err != nil {
		return IoNetSettings{}, err
	}
	if current.Revision != revision {
		return IoNetSettings{}, ErrConflict
	}
	if key != "" {
		if !validRepositoryToken(key) {
			return IoNetSettings{}, ErrInvalid
		}
		sealed, err = sealCredentialPurpose(s.credentials, ioNetSettingsPurpose, []byte(key))
		if err != nil {
			return IoNetSettings{}, err
		}
	} else if clear {
		sealed = ""
	}
	if enabled && sealed == "" {
		return IoNetSettings{}, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var next int64
	err = s.db.QueryRowContext(ctx, `UPDATE lmm_operations.ionet_config SET revision=revision+1,enabled=$1,api_key_ciphertext=$2,updated_by=$3 WHERE singleton AND revision=$4 RETURNING revision`, enabled, sealed, actor, revision).Scan(&next)
	if errors.Is(err, sql.ErrNoRows) {
		return IoNetSettings{}, ErrConflict
	}
	if err != nil {
		return IoNetSettings{}, ErrUnavailable
	}
	return IoNetSettings{Revision: next, Provider: "io.net", Enabled: enabled, Configured: sealed != "", CanConnect: enabled && sealed != "" && s.credentials != nil}, nil
}
func (s *IoNet) providerKey(ctx context.Context) (IoNetSettings, string, error) {
	settings, sealed, err := s.settings(ctx)
	if err != nil {
		return settings, "", err
	}
	if !settings.Enabled || !settings.Configured {
		return settings, "", ErrUnavailable
	}
	plain, err := openCredentialPurpose(s.credentials, ioNetSettingsPurpose, sealed)
	if err != nil || !validRepositoryToken(string(plain)) {
		return settings, "", ErrUnavailable
	}
	return settings, string(plain), nil
}
