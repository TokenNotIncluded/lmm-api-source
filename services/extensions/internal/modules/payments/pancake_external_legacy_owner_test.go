package payments

import (
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"database/sql"
	"database/sql/driver"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

func externalPancakeKeys(t *testing.T) (*pancake.WebhookPublicKeys, map[string]*rsa.PrivateKey) {
	t.Helper()
	keys := &pancake.WebhookPublicKeys{}
	private := map[string]*rsa.PrivateKey{}
	for _, mode := range []string{"test", "prod"} {
		key, err := rsa.GenerateKey(rand.Reader, 2048)
		if err != nil {
			t.Fatal(err)
		}
		public, err := x509.MarshalPKIXPublicKey(&key.PublicKey)
		if err != nil {
			t.Fatal(err)
		}
		encoded := string(pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: public}))
		if mode == "test" {
			keys.Test = encoded
		} else {
			keys.Prod = encoded
		}
		private[mode] = key
	}
	return keys, private
}

func externalPancakePayload(t *testing.T, mode, reference, product string) string {
	t.Helper()
	var event map[string]any
	if err := json.Unmarshal([]byte(pancakePaid), &event); err != nil {
		t.Fatal(err)
	}
	event["mode"] = mode
	data := event["data"].(map[string]any)
	data["orderMerchantExternalId"] = reference
	if product == "" {
		delete(data, "orderMetadata")
	} else {
		data["orderMetadata"].(map[string]any)["lmm_product_id"] = product
	}
	raw, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}

func externalPancakeSignature(t *testing.T, key *rsa.PrivateKey, payload string, timestamp time.Time) string {
	t.Helper()
	stamp := strconv.FormatInt(timestamp.UnixMilli(), 10)
	hash := sha256.Sum256([]byte(stamp + "." + payload))
	signature, err := rsa.SignPKCS1v15(rand.Reader, key, crypto.SHA256, hash[:])
	if err != nil {
		t.Fatal(err)
	}
	return "t=" + stamp + ",v1=" + base64.StdEncoding.EncodeToString(signature)
}

// Any attempted inbox connection fails. The early ACK must succeed with zero
// attempts; no PostgreSQL, durable-write success mock, or Core settlement exists.
type externalPancakeInbox struct{ connects atomic.Int32 }

func (p *externalPancakeInbox) Connect(context.Context) (driver.Conn, error) {
	p.connects.Add(1)
	return nil, errors.New("inbox unavailable in routing test")
}
func (*externalPancakeInbox) Driver() driver.Driver { return externalPancakeDriver{} }

type externalPancakeDriver struct{}

func (externalPancakeDriver) Open(string) (driver.Conn, error) {
	return nil, errors.New("inbox unavailable in routing test")
}

type externalPancakeCore struct {
	*legacyFake
	lookups int
}

func (c *externalPancakeCore) LookupLegacyProof(ctx context.Context, callback LegacyCallback) (LegacyProof, error) {
	c.lookups++
	return c.legacyFake.LookupLegacyProof(ctx, callback)
}

type externalPancakeFixture struct {
	subscriptions *PancakeSubscriptions
	wallet        *Service
	core          *externalPancakeCore
	store         *memoryStore
	inbox         *externalPancakeInbox
}

func newExternalPancakeFixture(t *testing.T, mode string) externalPancakeFixture {
	t.Helper()
	environment := "test"
	if mode == "prod" {
		environment = "live"
	}
	channel := Channel{ID: 4, Provider: "waffo_pancake", Merchant: "merchant", Environment: environment, Currency: "USD"}
	adapter := &Pancake{cfg: PancakeConfig{Channel: channel, StoreID: "STO_0123456789abcdefghijkl", ProductID: "shared_product"}}
	core := &externalPancakeCore{legacyFake: &legacyFake{}}
	store := newMemory()
	wallet, err := New(store, core, adapter)
	if err != nil {
		t.Fatal(err)
	}
	inbox := &externalPancakeInbox{}
	db := sql.OpenDB(inbox)
	t.Cleanup(func() { _ = db.Close() })
	subscriptions := &PancakeSubscriptions{db: db, adapters: map[int64]*Pancake{4: adapter}}
	subscriptions.AttachWallet(wallet)
	return externalPancakeFixture{subscriptions, wallet, core, store, inbox}
}

func (f externalPancakeFixture) assertUntouched(t *testing.T) {
	t.Helper()
	if f.inbox.connects.Load() != 0 || f.core.lookups != 0 || f.core.accepts != 0 || f.core.credits != 0 ||
		len(f.store.events) != 0 || len(f.store.orders) != 0 {
		t.Fatal("external callback reached an inbox, Core or wallet state")
	}
}

func externalPancakeRule(mode string, nativeProducts ...string) ExternalLegacyPancakeOwner {
	environment := "test"
	if mode == "prod" {
		environment = "live"
	}
	return ExternalLegacyPancakeOwner{StoreID: "STO_0123456789abcdefghijkl", Environment: environment, NativeProductIDs: nativeProducts}
}

func externalPancakeDeliver(handler http.Handler, pathMode, payload, signature string, duplicateHeader bool) *httptest.ResponseRecorder {
	request := httptest.NewRequest(http.MethodPost, "/api/waffo-pancake/webhook/"+pathMode, strings.NewReader(payload))
	request.SetPathValue("environment", pathMode)
	request.Header.Set("X-Waffo-Signature", signature)
	if duplicateHeader {
		request.Header.Add("X-Waffo-Signature", signature)
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	return response
}

func TestExternalLegacyPancakeSignedCallbackAcknowledgesBeforeInbox(t *testing.T) {
	keys, private := externalPancakeKeys(t)
	for _, mode := range []string{"test", "prod"} {
		for _, reference := range []string{"WAFFO_PANCAKE-123", "WAFFO_PANCAKE_SUB-123"} {
			for _, product := range []string{"shared_product", ""} {
				t.Run(mode+"/"+reference+"/"+product, func(t *testing.T) {
					f := newExternalPancakeFixture(t, mode)
					// A shared SKU needs no exclusive native product list.
					rule := externalPancakeRule(mode)
					if err := ValidateExternalLegacyPancakeOwners([]ExternalLegacyPancakeOwner{rule}); err != nil {
						t.Fatal(err)
					}
					payload := externalPancakePayload(t, mode, reference, product)
					signature := externalPancakeSignature(t, private[mode], payload, time.Now())
					handler := f.subscriptions.legacyWebhookHandler(f.wallet, keys, rule)
					for attempt := 0; attempt < 2; attempt++ {
						response := externalPancakeDeliver(handler, mode, payload, signature, false)
						if response.Code != http.StatusOK || response.Body.String() != "OK" {
							t.Fatalf("signed external callback: %d %s", response.Code, response.Body.String())
						}
					}
					f.assertUntouched(t)
				})
			}
		}
	}
}

func TestExternalLegacyPancakeRejectsSignatureAndExclusiveSKUConflicts(t *testing.T) {
	keys, private := externalPancakeKeys(t)
	for _, tc := range []struct {
		name, mode, signingMode, pathMode, product string
		age                                        time.Duration
		tamper, duplicateHeader, duplicateJSON     bool
		status                                     int
	}{
		{name: "exclusive_native_sku", mode: "test", product: "native_only", status: http.StatusConflict},
		{name: "prod_requires_prod_key", mode: "prod", signingMode: "test", status: http.StatusBadRequest},
		{name: "test_requires_test_key", mode: "test", signingMode: "prod", status: http.StatusBadRequest},
		{name: "changed_signed_bytes", mode: "test", tamper: true, status: http.StatusBadRequest},
		{name: "expired_signature", mode: "test", age: -46 * time.Minute, status: http.StatusBadRequest},
		{name: "future_signature", mode: "test", age: 2 * time.Minute, status: http.StatusBadRequest},
		{name: "wrong_path_environment", mode: "prod", pathMode: "test", status: http.StatusBadRequest},
		{name: "duplicate_signature", mode: "test", duplicateHeader: true, status: http.StatusBadRequest},
		{name: "duplicate_signed_reference", mode: "test", duplicateJSON: true, status: http.StatusBadRequest},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f := newExternalPancakeFixture(t, tc.mode)
			product := tc.product
			if product == "" {
				product = "shared_product"
			}
			payload := externalPancakePayload(t, tc.mode, "WAFFO_PANCAKE-123", product)
			if tc.duplicateJSON {
				payload = strings.Replace(payload, "\"orderMerchantExternalId\":", "\"orderMerchantExternalId\":\"top_1\",\"orderMerchantExternalId\":", 1)
			}
			signingMode := tc.signingMode
			if signingMode == "" {
				signingMode = tc.mode
			}
			signature := externalPancakeSignature(t, private[signingMode], payload, time.Now().Add(tc.age))
			if tc.tamper {
				payload = strings.Replace(payload, "ORD_fixture", "ORD_tampered", 1)
			}
			pathMode := tc.pathMode
			if pathMode == "" {
				pathMode = tc.mode
			}
			handler := f.subscriptions.legacyWebhookHandler(f.wallet, keys, externalPancakeRule(tc.mode, "native_only"))
			response := externalPancakeDeliver(handler, pathMode, payload, signature, tc.duplicateHeader)
			if response.Code != tc.status {
				t.Fatalf("status %d, want %d: %s", response.Code, tc.status, response.Body.String())
			}
			f.assertUntouched(t)
		})
	}
}

func TestExternalLegacyPancakePublicHandlerUsesOfficialKeys(t *testing.T) {
	_, private := externalPancakeKeys(t)
	f := newExternalPancakeFixture(t, "test")
	payload := externalPancakePayload(t, "test", "WAFFO_PANCAKE-123", "shared_product")
	signature := externalPancakeSignature(t, private["test"], payload, time.Now())
	response := externalPancakeDeliver(f.subscriptions.LegacyWebhookHandler(f.wallet, externalPancakeRule("test")),
		"test", payload, signature, false)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("public handler accepted a non-provider signing key: %d", response.Code)
	}
	f.assertUntouched(t)
}

func TestExternalLegacyPancakeRulesDoNotSwallowOtherOwnership(t *testing.T) {
	keys, private := externalPancakeKeys(t)
	for _, name := range []string{"absent", "other_store", "other_environment", "unknown_namespace", "empty_namespace", "other_event"} {
		t.Run(name, func(t *testing.T) {
			f := newExternalPancakeFixture(t, "test")
			// The original route must still ask Core. This fixture explicitly denies
			// legacy authority; a routing rule must not manufacture a successful ACK.
			f.wallet.pancakeRecovery = nil
			f.core.lookupErr = ErrDenied
			rule := externalPancakeRule("test")
			rules := []ExternalLegacyPancakeOwner{rule}
			reference := "WAFFO_PANCAKE-123"
			switch name {
			case "absent":
				rules = nil
			case "other_store":
				rules[0].StoreID = "another_store"
			case "other_environment":
				rules[0].Environment = "live"
			case "unknown_namespace":
				reference = "unknown_order"
			case "empty_namespace":
				reference = "WAFFO_PANCAKE-"
			}
			payload := externalPancakePayload(t, "test", reference, "shared_product")
			if name == "other_event" {
				payload = strings.Replace(payload, "order.completed", "refund.succeeded", 1)
			}
			signature := externalPancakeSignature(t, private["test"], payload, time.Now())
			response := externalPancakeDeliver(f.subscriptions.legacyWebhookHandler(f.wallet, keys, rules...),
				"test", payload, signature, false)
			if response.Code != http.StatusForbidden || f.core.lookups != 1 || f.core.accepts != 0 || f.core.credits != 0 {
				t.Fatalf("legacy authority bypassed: status=%d lookups=%d accepts=%d credits=%d",
					response.Code, f.core.lookups, f.core.accepts, f.core.credits)
			}
		})
	}
}

func TestExternalLegacyPancakeDefaultRetainsInboxAndLegacySettlement(t *testing.T) {
	keys, private := externalPancakeKeys(t)
	payload := externalPancakePayload(t, "test", "WAFFO_PANCAKE-123", "shared_product")
	signature := externalPancakeSignature(t, private["test"], payload, time.Now())
	t.Run("durable_inbox_failure_remains_retryable", func(t *testing.T) {
		f := newExternalPancakeFixture(t, "test")
		response := externalPancakeDeliver(f.subscriptions.LegacyWebhookHandler(f.wallet), "test", payload, signature, false)
		if response.Code != http.StatusServiceUnavailable || f.inbox.connects.Load() != 1 || f.core.lookups != 0 {
			t.Fatal("default legacy route bypassed its durable inbox", response.Code, f.inbox.connects.Load())
		}
	})
	t.Run("reviewed_legacy_core_handoff_preserved", func(t *testing.T) {
		f := newExternalPancakeFixture(t, "test")
		f.wallet.pancakeRecovery = nil
		handler := f.subscriptions.legacyWebhookHandler(f.wallet, keys)
		for attempt := 0; attempt < 2; attempt++ {
			response := externalPancakeDeliver(handler, "test", payload, signature, false)
			if response.Code != http.StatusOK {
				t.Fatal(response.Code, response.Body.String())
			}
		}
		if f.core.lookups != 2 || f.core.accepts != 1 || f.core.credits != 2 || f.core.postings != 1 ||
			f.core.raw != payload || f.core.sig != signature {
			t.Fatal("default legacy proof, replay or original bytes changed")
		}
	})
}

func TestExternalLegacyPancakeUnknownNativeReferencesRemainRetryable(t *testing.T) {
	keys, private := externalPancakeKeys(t)
	for _, reference := range []string{"top_2", "sub_unknown"} {
		t.Run(reference, func(t *testing.T) {
			f := newExternalPancakeFixture(t, "test")
			payload := externalPancakePayload(t, "test", reference, "native_only")
			signature := externalPancakeSignature(t, private["test"], payload, time.Now())
			response := externalPancakeDeliver(f.subscriptions.legacyWebhookHandler(f.wallet, keys,
				externalPancakeRule("test", "native_only")), "test", payload, signature, false)
			if reference == "top_2" {
				if response.Code != http.StatusNotFound || f.inbox.connects.Load() != 0 {
					t.Fatal("unknown top_ was not kept on native lookup", response.Code)
				}
			} else if response.Code != http.StatusServiceUnavailable || f.inbox.connects.Load() != 1 {
				t.Fatal("sub_ bypassed its original inbox/retry path", response.Code)
			}
			if f.core.lookups != 0 || f.core.accepts != 0 || f.core.credits != 0 {
				t.Fatal("native reference fell back to legacy Core")
			}
		})
	}
}
