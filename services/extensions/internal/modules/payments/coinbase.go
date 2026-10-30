// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"net/http"
	"strconv"
	"strings"
	"time"
)

const coinbaseMaxMinor int64 = 10_000_000_000

// Coinbase uses Business Checkouts, not the retired Commerce charges API.
// The wallet quotes USD and explicitly charges the same decimal amount in USDC.
// Only Core holds the independently provisioned webhook subscription secret.
type CoinbaseConfig struct {
	Channel    Channel
	APIKey     string // CDP API key name, not the private key or a Commerce key.
	PrivateKey string // ES256 / P-256 PEM.
	SuccessURL string
	CancelURL  string
}

type Coinbase struct {
	cfg    CoinbaseConfig
	key    *ecdsa.PrivateKey
	client *http.Client
}

func NewCoinbase(cfg CoinbaseConfig) (*Coinbase, error) {
	if !cfg.Channel.valid() || cfg.Channel.Provider != "coinbase" || cfg.Channel.Currency != "USD" || !validProviderCredential(cfg.APIKey) {
		return nil, ErrInvalid
	}
	for _, target := range []string{cfg.SuccessURL, cfg.CancelURL} {
		if _, err := secureURL(target); err != nil || len(target) > 2048 {
			return nil, ErrInvalid
		}
	}
	block, rest := pem.Decode([]byte(cfg.PrivateKey))
	if block == nil || len(strings.TrimSpace(string(rest))) != 0 {
		return nil, ErrInvalid
	}
	var key *ecdsa.PrivateKey
	var err error
	switch block.Type {
	case "EC PRIVATE KEY":
		key, err = x509.ParseECPrivateKey(block.Bytes)
	case "PRIVATE KEY":
		var parsed any
		parsed, err = x509.ParsePKCS8PrivateKey(block.Bytes)
		key, _ = parsed.(*ecdsa.PrivateKey)
	default:
		return nil, ErrInvalid
	}
	if err != nil || key == nil || key.Curve != elliptic.P256() {
		return nil, ErrInvalid
	}
	cfg.PrivateKey = ""
	return &Coinbase{cfg: cfg, key: key, client: outboundClient()}, nil
}

func (c *Coinbase) Channel() Channel                { return c.cfg.Channel }
func (*Coinbase) RefundEnabled() bool               { return false }
func (*Coinbase) PartialRefunds() bool              { return false }
func (*Coinbase) ReplayWindow(string) time.Duration { return 0 }
func (*Coinbase) Refund(context.Context, Order, Refund, string) (RefundResult, error) {
	return RefundResult{}, ErrUnsupported
}

// Coinbase requires a random UUIDv4. Create persists it as the checkout job key
// before any provider I/O; duplicate creates return the first stored order/key.
func coinbaseCheckoutKey() (string, error) {
	var key [16]byte
	if _, err := rand.Read(key[:]); err != nil {
		return "", ErrUnavailable
	}
	key[6] = key[6]&0x0f | 0x40
	key[8] = key[8]&0x3f | 0x80
	s := hex.EncodeToString(key[:])
	return s[:8] + "-" + s[8:12] + "-" + s[12:16] + "-" + s[16:20] + "-" + s[20:], nil
}

func coinbaseUUID(key string) bool {
	if len(key) != 36 || key[14] != '4' || !strings.ContainsRune("89ab", rune(key[19])) {
		return false
	}
	for i, b := range []byte(key) {
		if i == 8 || i == 13 || i == 18 || i == 23 {
			if b != '-' {
				return false
			}
		} else if !(b >= '0' && b <= '9' || b >= 'a' && b <= 'f') {
			return false
		}
	}
	return true
}

// Coinbase's documented JWT contract: a fresh, short-lived ES256 token bound to
// the exact method, host and path, including /sandbox for test requests.
func (c *Coinbase) token(method, path string) (string, error) {
	var nonce [16]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return "", ErrUnavailable
	}
	now := time.Now().Unix()
	header, err := json.Marshal(map[string]string{"alg": "ES256", "typ": "JWT", "kid": c.cfg.APIKey, "nonce": hex.EncodeToString(nonce[:])})
	if err != nil {
		return "", ErrInvalid
	}
	claims, err := json.Marshal(map[string]any{"iss": "cdp", "sub": c.cfg.APIKey, "nbf": now, "exp": now + 120, "uri": method + " business.coinbase.com" + path})
	if err != nil {
		return "", ErrInvalid
	}
	unsigned := base64.RawURLEncoding.EncodeToString(header) + "." + base64.RawURLEncoding.EncodeToString(claims)
	digest := sha256.Sum256([]byte(unsigned))
	r, s, err := ecdsa.Sign(rand.Reader, c.key, digest[:])
	if err != nil {
		return "", ErrUnavailable
	}
	// JWS ES256 uses fixed-width R || S, not an ASN.1 ECDSA signature.
	var signature [64]byte
	r.FillBytes(signature[:32])
	s.FillBytes(signature[32:])
	return unsigned + "." + base64.RawURLEncoding.EncodeToString(signature[:]), nil
}

func (c *Coinbase) api(ctx context.Context, method, suffix, key string, value any) ([]byte, error) {
	path := "/api/v1/checkouts" + suffix
	if c.cfg.Channel.Environment == "test" {
		path = "/sandbox" + path
	}
	token, err := c.token(method, path)
	if err != nil {
		return nil, err
	}
	headers := http.Header{"Authorization": {"Bearer " + token}}
	if key != "" {
		headers.Set("X-Idempotency-Key", key)
	}
	return requestJSONObject(ctx, c.client, method, "https://business.coinbase.com"+path, value, headers)
}

type coinbaseCheckout struct {
	ID        string            `json:"id"`
	URL       string            `json:"url"`
	Amount    string            `json:"amount"`
	Currency  string            `json:"currency"`
	Status    string            `json:"status"`
	EventType string            `json:"eventType"`
	Metadata  map[string]string `json:"metadata"`
}

func (c *Coinbase) binding(v coinbaseCheckout) (int64, error) {
	minor, err := minorDecimal(v.Amount)
	order := v.Metadata["lmm_order_id"]
	if err != nil || minor > coinbaseMaxMinor || !validID(v.ID) || v.Currency != "USDC" ||
		!validTopupOrderID(order) || v.Metadata["lmm_core_intent_id"] != strings.TrimPrefix(order, "top_") ||
		v.Metadata["lmm_merchant"] != c.cfg.Channel.Merchant || v.Metadata["lmm_environment"] != c.cfg.Channel.Environment {
		return 0, ErrConflict
	}
	return minor, nil
}

func (c *Coinbase) Checkout(ctx context.Context, o Order, key string) (Checkout, error) {
	if !providerOrderMatches(o, c.cfg.Channel) || o.Intent.AmountMinor > coinbaseMaxMinor || !coinbaseUUID(key) {
		return Checkout{}, ErrInvalid
	}
	raw, err := c.api(ctx, http.MethodPost, "", key, map[string]any{
		"amount": decimalMinor(o.Intent.AmountMinor), "currency": "USDC", "description": "LMM API wallet top-up " + o.ID,
		"successRedirectUrl": c.cfg.SuccessURL, "failRedirectUrl": c.cfg.CancelURL,
		"metadata": map[string]string{"lmm_order_id": o.ID, "lmm_core_intent_id": strconv.FormatInt(o.Intent.ID, 10), "lmm_merchant": o.Intent.Merchant, "lmm_environment": o.Intent.Environment},
	})
	if err != nil {
		return Checkout{}, err
	}
	var v coinbaseCheckout
	if jsonObject(raw, &v) != nil {
		return Checkout{}, ErrConflict
	}
	minor, err := c.binding(v)
	if err != nil || minor != o.Intent.AmountMinor || v.Metadata["lmm_order_id"] != o.ID || v.Status != "ACTIVE" {
		return Checkout{}, ErrConflict
	}
	u, err := secureURL(v.URL)
	if err != nil || u.Hostname() != "payments.coinbase.com" || u.Fragment != "" {
		return Checkout{}, ErrConflict
	}
	return Checkout{ID: v.ID, URL: v.URL, PaymentID: v.ID}, nil
}

// Verify parses an observation. Core independently authenticates these original
// bytes with v0 before Service.Receive acknowledges it or queues wallet credit.
func (c *Coinbase) Verify(r *http.Request, _ time.Time) (Event, error) {
	header := "X-Webhook-Signature"
	current, legacy := r.Header.Values(header), r.Header.Values("X-Hook0-Signature")
	if len(current) > 1 || len(legacy) > 1 || len(current) == 1 && len(legacy) == 1 && current[0] != legacy[0] {
		return Event{}, ErrInvalid
	}
	if len(current) == 0 {
		header = "X-Hook0-Signature"
	}
	raw, signature, err := callbackBytes(r, header)
	if err != nil {
		return Event{}, err
	}
	var v coinbaseCheckout
	if jsonObject(raw, &v) != nil {
		return Event{}, ErrInvalid
	}
	if v.EventType != "checkout.payment.success" {
		return Event{}, ErrUnsupported
	}
	e, err := c.payment(v)
	if err != nil {
		return Event{}, err
	}
	e.Evidence.EventID = stableKey(v.ID, v.EventType)
	e.Evidence.Payload, e.Evidence.Signature, e.Evidence.Source = raw, signature, "webhook"
	return e, nil
}

func (c *Coinbase) payment(v coinbaseCheckout) (Event, error) {
	minor, err := c.binding(v)
	if err != nil || v.Status != "COMPLETED" {
		return Event{}, ErrConflict
	}
	ch := c.cfg.Channel
	return Event{OrderID: v.Metadata["lmm_order_id"], Type: Paid, Currency: ch.Currency, AmountMinor: minor,
		Evidence: Evidence{Provider: ch.Provider, Merchant: ch.Merchant, Environment: ch.Environment, Transaction: v.ID}}, nil
}

func (c *Coinbase) Lookup(ctx context.Context, o Order) ([]Event, error) {
	if !providerOrderMatches(o, c.cfg.Channel) || !validID(o.Checkout.ID) {
		return nil, ErrUnsupported
	}
	raw, err := c.api(ctx, http.MethodGet, "/"+o.Checkout.ID, "", nil)
	if err != nil {
		return nil, err
	}
	var v coinbaseCheckout
	if jsonObject(raw, &v) != nil {
		return nil, ErrConflict
	}
	minor, err := c.binding(v)
	if err != nil || v.ID != o.Checkout.ID || v.Metadata["lmm_order_id"] != o.ID || minor != o.Intent.AmountMinor {
		return nil, ErrConflict
	}
	if v.Status != "COMPLETED" {
		return nil, nil
	}
	e, err := c.payment(v)
	if err != nil {
		return nil, err
	}
	e.Evidence.Payload = raw
	apiEvidence(&e, v.ID, Paid)
	return []Event{e}, nil
}
