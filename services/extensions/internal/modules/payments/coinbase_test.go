package payments

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

const coinbasePaid = `{"id":"69163c762331ed43dc64a6ef","amount":"10.00","currency":"USDC","eventType":"checkout.payment.success","status":"COMPLETED","metadata":{"lmm_order_id":"top_1","lmm_core_intent_id":"1","lmm_merchant":"merchant_fixture","lmm_environment":"test"},"settlement":{"totalAmount":"10.00","feeAmount":"0.10","netAmount":"9.90","currency":"USDC"}}`

func coinbaseFixture(t *testing.T) *Coinbase {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	c, err := NewCoinbase(CoinbaseConfig{
		Channel: Channel{ID: 8, Provider: "coinbase", Merchant: "merchant_fixture", Environment: "test", Currency: "USD", Enabled: true},
		APIKey:  "organizations/test/apiKeys/test-key", PrivateKey: string(pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: der})),
		SuccessURL: "https://example.test/console/wallet", CancelURL: "https://example.test/console/wallet",
	})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func coinbaseJWTValid(t *testing.T, r *http.Request, c *Coinbase) {
	t.Helper()
	parts := strings.Split(strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), ".")
	if len(parts) != 3 {
		t.Error("missing request JWT")
		return
	}
	var header map[string]string
	var claims struct {
		Issuer    string `json:"iss"`
		Subject   string `json:"sub"`
		URI       string `json:"uri"`
		NotBefore int64  `json:"nbf"`
		Expires   int64  `json:"exp"`
	}
	h, herr := base64.RawURLEncoding.DecodeString(parts[0])
	b, berr := base64.RawURLEncoding.DecodeString(parts[1])
	sig, serr := base64.RawURLEncoding.DecodeString(parts[2])
	if herr != nil || berr != nil || serr != nil || json.Unmarshal(h, &header) != nil || json.Unmarshal(b, &claims) != nil || len(sig) != 64 {
		t.Error("invalid compact JWS")
		return
	}
	digest := sha256.Sum256([]byte(parts[0] + "." + parts[1]))
	if !ecdsa.Verify(&c.key.PublicKey, digest[:], new(big.Int).SetBytes(sig[:32]), new(big.Int).SetBytes(sig[32:])) ||
		header["alg"] != "ES256" || header["kid"] != c.cfg.APIKey || len(header["nonce"]) != 32 ||
		claims.Issuer != "cdp" || claims.Subject != c.cfg.APIKey || claims.URI != r.Method+" business.coinbase.com"+r.URL.Path ||
		claims.Expires-claims.NotBefore != 120 || claims.NotBefore > time.Now().Unix() || claims.Expires <= time.Now().Unix() {
		t.Error("JWT signature, request binding or lifetime differs from Coinbase contract")
	}
}

func TestCoinbaseCheckoutAndLookupUseBoundJWTAndUSDC(t *testing.T) {
	for _, environment := range []string{"test", "live"} {
		t.Run(environment, func(t *testing.T) {
			c := coinbaseFixture(t)
			c.cfg.Channel.Environment = environment
			key, err := coinbaseCheckoutKey()
			if err != nil {
				t.Fatal(err)
			}
			var requests atomic.Int32
			var previousJWT string
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				requests.Add(1)
				coinbaseJWTValid(t, r, c)
				if previousJWT == r.Header.Get("Authorization") {
					t.Error("JWT reused across requests")
				}
				previousJWT = r.Header.Get("Authorization")
				path := "/api/v1/checkouts"
				if environment == "test" {
					path = "/sandbox" + path
				}
				if r.Method == http.MethodPost {
					var body map[string]json.RawMessage
					if json.NewDecoder(r.Body).Decode(&body) != nil || r.URL.Path != path || r.Header.Get("X-Idempotency-Key") != key || !coinbaseUUID(key) || string(body["currency"]) != `"USDC"` || string(body["amount"]) != `"10.00"` {
						t.Error("checkout changed path, amount, currency or durable key", r.URL.Path)
					}
					var metadata map[string]string
					if json.Unmarshal(body["metadata"], &metadata) != nil || metadata["lmm_order_id"] != "top_1" || metadata["lmm_core_intent_id"] != "1" || metadata["lmm_merchant"] != c.Channel().Merchant || metadata["lmm_environment"] != environment {
						t.Error("checkout lost frozen intent binding")
					}
					w.WriteHeader(http.StatusCreated)
					_ = json.NewEncoder(w).Encode(map[string]any{"id": "69163c762331ed43dc64a6ef", "url": "https://payments.coinbase.com/payment-links/pl_fixture", "amount": "10.00", "currency": "USDC", "status": "ACTIVE", "metadata": metadata})
					return
				}
				if r.Method != http.MethodGet || r.URL.Path != path+"/69163c762331ed43dc64a6ef" || r.Header.Get("X-Idempotency-Key") != "" {
					t.Error("wrong lookup request")
				}
				_, _ = io.WriteString(w, strings.Replace(coinbasePaid, `"lmm_environment":"test"`, `"lmm_environment":"`+environment+`"`, 1))
			}))
			defer server.Close()
			endpoint, _ := url.Parse(server.URL)
			c.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
				if r.URL.Scheme != "https" || r.URL.Host != "business.coinbase.com" {
					t.Error("provider origin changed")
				}
				local := r.Clone(r.Context())
				local.URL.Scheme, local.URL.Host = endpoint.Scheme, endpoint.Host
				return server.Client().Transport.RoundTrip(local)
			})}
			o := gatewayOrder(c.Channel())
			checkout, err := c.Checkout(context.Background(), o, key)
			if err != nil || checkout.ID != "69163c762331ed43dc64a6ef" || checkout.PaymentID != checkout.ID {
				t.Fatal(checkout, err)
			}
			o.Checkout, o.PaymentID = checkout, checkout.ID
			events, err := c.Lookup(context.Background(), o)
			if err != nil || len(events) != 1 || events[0].AmountMinor != 1000 || events[0].Currency != "USD" || events[0].Evidence.Source != "api" || len(events[0].Evidence.Signature) != 0 || events[0].Evidence.AcceptedProofID != "" || requests.Load() != 2 {
				t.Fatal("lookup forged signed proof or changed frozen amount", events, err)
			}
		})
	}
}

func TestCoinbaseCheckoutRejectsMismatchedProviderResponse(t *testing.T) {
	c := coinbaseFixture(t)
	key, _ := coinbaseCheckoutKey()
	responseBody := strings.Replace(coinbasePaid, `"status":"COMPLETED"`, `"status":"ACTIVE","url":"https://payments.coinbase.com/payment-links/pl_fixture"`, 1)
	for _, pair := range [][2]string{
		{`"amount":"10.00"`, `"amount":"10.01"`},
		{`"currency":"USDC"`, `"currency":"USD"`},
		{`"lmm_core_intent_id":"1"`, `"lmm_core_intent_id":"2"`},
		{`"lmm_environment":"test"`, `"lmm_environment":"live"`},
		{"https://payments.coinbase.com/", "https://payments.coinbase.com.evil.test/"},
		{"https://payments.coinbase.com/", "http://payments.coinbase.com/"},
		{"https://payments.coinbase.com/", "https://user@payments.coinbase.com/"},
	} {
		c.client = &http.Client{Transport: transportFunc(func(*http.Request) (*http.Response, error) {
			return response(strings.Replace(responseBody, pair[0], pair[1], 1)), nil
		})}
		if _, err := c.Checkout(context.Background(), gatewayOrder(c.Channel()), key); !errors.Is(err, ErrConflict) {
			t.Fatal("mismatched response accepted", pair, err)
		}
	}
}

func TestCoinbaseCallbackPreservesEvidenceAndRequiresCoreProof(t *testing.T) {
	c := coinbaseFixture(t)
	for _, headers := range [][]string{{"X-Webhook-Signature"}, {"X-Hook0-Signature"}, {"X-Webhook-Signature", "X-Hook0-Signature"}} {
		r := gatewayCallback(coinbasePaid, headers[0])
		for _, h := range headers {
			r.Header.Set(h, "original-provider-signature")
		}
		e, err := c.Verify(r, time.Now())
		if err != nil || string(e.Evidence.Payload) != coinbasePaid || string(e.Evidence.Signature) != "original-provider-signature" || e.Evidence.EventID != stableKey("69163c762331ed43dc64a6ef", "checkout.payment.success") || e.AmountMinor != 1000 || e.Currency != "USD" {
			t.Fatal("callback evidence or money changed", e, err)
		}
	}
	for _, raw := range []string{
		strings.Replace(coinbasePaid, "checkout.payment.success", "checkout.payment.failed", 1),
		strings.Replace(coinbasePaid, `"COMPLETED"`, `"ACTIVE"`, 1),
		strings.Replace(coinbasePaid, `"USDC"`, `"USD"`, 1),
		strings.Replace(coinbasePaid, `"amount":"10.00"`, `"amount":"10.001"`, 1),
		strings.Replace(coinbasePaid, `"lmm_core_intent_id":"1"`, `"lmm_core_intent_id":"2"`, 1),
		strings.Replace(coinbasePaid, `"lmm_environment":"test"`, `"lmm_environment":"live"`, 1),
		strings.Replace(coinbasePaid, `"amount":"10.00"`, `"amount":"0.00","amount":"10.00"`, 1),
	} {
		if _, err := c.Verify(gatewayCallback(raw, "X-Webhook-Signature"), time.Now()); err == nil {
			t.Fatal("invalid callback accepted", raw)
		}
	}
	conflict := gatewayCallback(coinbasePaid, "X-Webhook-Signature")
	conflict.Header.Set("X-Hook0-Signature", "different")
	if _, err := c.Verify(conflict, time.Now()); err == nil {
		t.Fatal("conflicting signature headers accepted")
	}
	memory, core := newMemory(), newCore(c.Channel())
	s, err := New(memory, callbackRejectProofCore{Core: core}, c)
	if err != nil {
		t.Fatal(err)
	}
	if err = memory.Insert(context.Background(), gatewayOrder(c.Channel())); err != nil {
		t.Fatal(err)
	}
	if err = s.Receive(context.Background(), 8, gatewayCallback(coinbasePaid, "X-Webhook-Signature")); !errors.Is(err, ErrEvidence) || len(memory.events) != 0 || core.count("credit") != 0 {
		t.Fatal("Core proof rejection bypassed", err)
	}
}

func TestCoinbaseCheckoutKeySurvivesDuplicateCreateAndUnknownResult(t *testing.T) {
	c := coinbaseFixture(t)
	memory, core := newMemory(), newCore(c.Channel())
	s, err := New(memory, core, c)
	if err != nil {
		t.Fatal(err)
	}
	request := PrepareRequest{Key: "coinbase_fixture_create_001", AccountID: 99, ChannelID: 8, Currency: "USD", AmountMinor: 1000}
	var group sync.WaitGroup
	for range 12 {
		group.Go(func() {
			if _, err := s.Create(context.Background(), "user", request); err != nil {
				t.Error(err)
			}
		})
	}
	group.Wait()
	o, err := memory.Get(context.Background(), "top_1")
	if err != nil || len(o.Jobs) != 1 {
		t.Fatal("duplicate create changed checkout jobs", o, err)
	}
	var original string
	for key := range o.Jobs {
		original = key
	}
	if !coinbaseUUID(original) {
		t.Fatal("checkout key is not UUIDv4", original)
	}
	var calls atomic.Int32
	c.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		calls.Add(1)
		if r.Header.Get("X-Idempotency-Key") != original {
			t.Error("provider key differs from durable key")
		}
		return nil, context.DeadlineExceeded
	})}
	if err = s.Work(context.Background(), 1); err != nil {
		t.Fatal(err)
	}
	// A new service instance recovers the original stored job. An ambiguous
	// provider POST is not repeated when retention is undocumented.
	s, err = New(memory, core, c)
	if err != nil {
		t.Fatal(err)
	}
	s.now = func() time.Time { return time.Now().Add(time.Hour) }
	if err = s.Work(context.Background(), 1); err != nil {
		t.Fatal(err)
	}
	o, err = memory.Get(context.Background(), "top_1")
	if err != nil || calls.Load() != 1 || len(o.Jobs) != 1 || o.Jobs[original].State != "review" || o.Credited() {
		t.Fatal("unknown checkout was retried or identity changed", o, err)
	}
}
