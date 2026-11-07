package store

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

const (
	extoreFlowKind       = "extore_flows"
	extoreConnectionKind = "extore_connections"
	extorePurpose        = "lmm-store-extore-v1"
)

type extoreCipher struct{ aead cipher.AEAD }

type extoreFlowRow struct {
	UserID    int64       `json:"user_id"`
	Binding   string      `json:"binding"`
	Flow      extore.Flow `json:"flow"`
	ExpiresAt int64       `json:"expires_at"`
	Consumed  bool        `json:"consumed"`
}

type extoreConnectionRow struct {
	ID             string `json:"id"`
	Origin         string `json:"origin"`
	ShopID         string `json:"shop_id"`
	ShopName       string `json:"shop_name"`
	ClientID       string `json:"client_id"`
	Ciphertext     string `json:"ciphertext"`
	ExpiresAt      int64  `json:"expires_at"`
	RefreshPending bool   `json:"refresh_pending"`
	Version        int64  `json:"version"`
	UpdatedAt      int64  `json:"updated_at"`
}

// WithExtore enables Extore authorize/catalog when redirect URI and a dedicated
// 32-byte seal key are both configured. The key is never shared with Core tokens.
func (s *Service) WithExtore(redirectURI string, key []byte) error {
	if s == nil || len(key) != 32 {
		return ErrInvalid
	}
	flow, err := extore.NewFlow(extore.DefaultOrigin, "lmm-best-store", redirectURI)
	if err != nil {
		return ErrInvalid
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return err
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return err
	}
	s.extoreRedirect = flow.RedirectURI
	s.extore = &extoreCipher{aead}
	s.extoreHTTP = &http.Client{Timeout: 20 * time.Second}
	return nil
}

func (s *Service) ExtoreReady() bool {
	return s != nil && s.extore != nil && s.extoreRedirect != ""
}

func extoreSellerShop(userID int64) string {
	return "extore:" + ID(extorePurpose, "shop", userID)[:48]
}

func (c *extoreCipher) seal(aad string, plain []byte) (string, error) {
	nonce := make([]byte, c.aead.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return "", err
	}
	return "v1:" + base64.RawURLEncoding.EncodeToString(c.aead.Seal(nonce, nonce, plain, []byte(aad))), nil
}

func (c *extoreCipher) open(aad, value string) ([]byte, error) {
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(value, "v1:"))
	if !strings.HasPrefix(value, "v1:") || err != nil || len(raw) < c.aead.NonceSize() {
		return nil, ErrInvalid
	}
	nonce, sealed := raw[:c.aead.NonceSize()], raw[c.aead.NonceSize():]
	return c.aead.Open(nil, nonce, sealed, []byte(aad))
}

func extoreSessionBinding(token string) string {
	return ID(extorePurpose, "session", token)
}

func mapExtoreErr(err error) error {
	switch {
	case err == nil:
		return nil
	case errors.Is(err, extore.ErrInput):
		return ErrInvalid
	case errors.Is(err, extore.ErrDenied):
		return ErrForbidden
	case errors.Is(err, extore.ErrProtocol), errors.Is(err, extore.ErrUnavailable):
		return ErrUnavailable
	default:
		return ErrUnavailable
	}
}

// ExtoreAuthorize starts a PKCE authorize flow bound to the current session.
func (s *Service) ExtoreAuthorize(ctx context.Context, token string, userID int64, baseURL, clientID string) (map[string]any, error) {
	if !s.ExtoreReady() {
		return nil, ErrUnavailable
	}
	if userID <= 0 || !Text(token, 256) {
		return nil, ErrUnauthorized
	}
	if _, err := s.check(ctx, token, Account{Kind: "personal", ID: userID}, Read); err != nil {
		return nil, err
	}
	if clientID == "" {
		clientID = "lmm-best-store"
	}
	flow, err := extore.NewFlow(baseURL, clientID, s.extoreRedirect)
	if err != nil {
		return nil, mapExtoreErr(err)
	}
	client := extore.Client{HTTP: s.extoreHTTP}
	if err := client.Discover(ctx, flow.Origin); err != nil {
		return nil, mapExtoreErr(err)
	}
	state := ID(extorePurpose, "state", userID, time.Now().UnixNano(), flow.Verifier)
	row := extoreFlowRow{
		UserID:    userID,
		Binding:   extoreSessionBinding(token),
		Flow:      flow,
		ExpiresAt: time.Now().UTC().Unix() + 10*60,
	}
	shop := extoreSellerShop(userID)
	err = s.repo.Within(ctx, shop, func(tx Tx) error {
		return tx.Put(extoreFlowKind, state, row)
	})
	if err != nil {
		return nil, err
	}
	return map[string]any{
		"authorization_url": flow.SyncAuthorizationURL(state),
		"base_url":          flow.Origin,
		"redirect_uri":      flow.RedirectURI,
	}, nil
}

// ExtoreCatalog consumes a one-time callback, persists the connection, and
// returns the Extore commerce catalog for the draft-import UI.
func (s *Service) ExtoreCatalog(ctx context.Context, token string, userID int64, callbackURL string) (any, error) {
	if !s.ExtoreReady() {
		return nil, ErrUnavailable
	}
	if userID <= 0 || !Text(token, 256) {
		return nil, ErrUnauthorized
	}
	if _, err := s.check(ctx, token, Account{Kind: "personal", ID: userID}, Read); err != nil {
		return nil, err
	}
	_, query, err := extore.ParseCallback(callbackURL)
	if err != nil {
		return nil, mapExtoreErr(err)
	}
	state := query.Get("state")
	if !Text(state, 128) {
		return nil, ErrInvalid
	}
	shop := extoreSellerShop(userID)
	binding := extoreSessionBinding(token)
	var flow extore.Flow
	err = s.repo.Within(ctx, shop, func(tx Tx) error {
		var row extoreFlowRow
		if err := tx.Get(extoreFlowKind, state, &row); err != nil {
			return err
		}
		if row.Consumed || row.UserID != userID || row.Binding != binding || row.ExpiresAt <= time.Now().UTC().Unix() {
			return ErrConflict
		}
		row.Consumed = true
		if err := tx.Put(extoreFlowKind, state, row); err != nil {
			return err
		}
		flow = row.Flow
		return nil
	})
	if err != nil {
		return nil, err
	}
	code, callbackErr := flow.ValidateCallback(callbackURL)
	if callbackErr != nil {
		return nil, mapExtoreErr(callbackErr)
	}
	client := extore.Client{HTTP: s.extoreHTTP}
	credentials, err := client.Connect(ctx, flow, code)
	if err != nil {
		return nil, mapExtoreErr(err)
	}
	catalog, err := client.ConnectionCatalog(ctx, flow.Origin, *credentials)
	if err != nil {
		return nil, mapExtoreErr(err)
	}
	secret, err := json.Marshal(credentials)
	if err != nil {
		return nil, ErrUnavailable
	}
	cipher, err := s.extore.seal(extorePurpose+"\x00connection\x00"+catalog.Shop.ID, secret)
	if err != nil {
		return nil, ErrUnavailable
	}
	connectionID := ID(extorePurpose, "connection", userID, flow.Origin, catalog.Shop.ID)
	now := time.Now().UTC().Unix()
	err = s.repo.Within(ctx, shop, func(tx Tx) error {
		var existing extoreConnectionRow
		version := int64(1)
		switch err := tx.Get(extoreConnectionKind, connectionID, &existing); {
		case err == nil:
			version = existing.Version + 1
		case errors.Is(err, ErrNotFound):
		default:
			return err
		}
		return tx.Put(extoreConnectionKind, connectionID, extoreConnectionRow{
			ID: connectionID, Origin: flow.Origin, ShopID: catalog.Shop.ID, ShopName: catalog.Shop.Name,
			ClientID: flow.ClientID, Ciphertext: cipher, ExpiresAt: int64(credentials.GrantExpires),
			Version: version, UpdatedAt: now,
		})
	})
	if err != nil {
		return nil, err
	}
	return catalog, nil
}
