// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package herosms

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
)

const syntheticServiceToken = "synthetic-hero-service-token-0000000000000000"
const syntheticSession = "lmms_synthetic_user_session_00000000000000000"

// This is a real local TCP HTTP contract fixture, not Unix deployment, provider
// or ledger evidence. Production NewCore retains its Unix-only connector.
// The managed execution environment rejected the initial Unix-socket attempt;
// that separate deployment check remains outstanding.
func serviceFixture(t *testing.T, handler http.HandlerFunc) *Service {
	t.Helper()
	socket := filepath.Join(t.TempDir(), "core.sock")
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer "+syntheticServiceToken || r.Header.Get("X-LMM-Protocol") != "1" {
			t.Error("missing separate service authority")
			http.Error(w, "forbidden", 403)
			return
		}
		if r.URL.Path != "/internal/hero-sms/v1/ready" && r.Header.Get("X-LMM-User-Credential") != syntheticSession {
			w.WriteHeader(403)
			_, _ = io.WriteString(w, `{"error":"FORBIDDEN"}`)
			return
		}
		handler(w, r)
	})}
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(func() { _ = server.Close() })
	core, err := NewCore(socket, []byte(syntheticServiceToken))
	if err != nil {
		t.Fatal(err)
	}
	core.client.Transport.(*http.Transport).DialContext = func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", listener.Addr().String())
	}
	t.Cleanup(core.Close)
	return &Service{Core: core}
}
func webRequest(t *testing.T, s *Service, method, path, body, key string) (int, map[string]any) {
	t.Helper()
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	r.Header.Set("X-LMM-User-Credential", syntheticSession)
	if key != "" {
		r.Header.Set("Idempotency-Key", key)
	}
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	var out map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &out); err != nil {
		t.Fatalf("bad response %s", w.Body.String())
	}
	if w.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("activation response must not be cached")
	}
	return w.Code, out
}
func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}
func syntheticReceipt() map[string]any {
	return map[string]any{"id": "order1", "quote_id": "quote1", "state": "reserved", "dispatched": true, "provider_id": "501", "usable": true, "charge_quota": 150000, "refunded_quota": 0, "created_at": 100, "updated_at": 100, "quote": map[string]any{"id": "quote1", "selection": map[string]any{"kind": "sms", "country": 6, "service": "tg"}, "charge_quota": 150000, "customer_price_usd": "0.300000", "supplier_cost_usd": "0.030000", "credential_sha256": "private-fingerprint"}, "activation": map[string]any{"phone_number": "15550000123", "code": "", "evidence": "private-proof", "cost_usd": "0.03"}}
}

func TestHeroSMSBidAndReceiptProjectionDoNotExposeSupplierFacts(t *testing.T) {
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		var in map[string]any
		_ = json.NewDecoder(r.Body).Decode(&in)
		switch r.URL.Path {
		case "/internal/hero-sms/v1/quote":
			if in["max_customer_price_usd"] != "0.6" {
				t.Error("customer maximum must reach Core quote policy")
			}
			writeJSON(w, map[string]any{"id": "quote1", "selection": map[string]any{"kind": "sms", "country": 6, "service": "tg"}, "customer_price_usd": "0.600000", "charge_quota": 300000, "inventory": 5, "bid": true, "supplier_cost_usd": "0.060000", "credential_sha256": "private-fingerprint", "tiers": []any{map[string]any{"customer_price_usd": "0.300000", "charge_quota": 150000, "inventory": 3}}})
		case "/internal/hero-sms/v1/poll":
			writeJSON(w, syntheticReceipt())
		case "/internal/hero-sms/v1/balance":
			writeJSON(w, map[string]any{"quota": 4350000})
		default:
			t.Error("unexpected action", r.URL.Path)
			http.NotFound(w, r)
		}
	})
	code, out := webRequest(t, s, "GET", "/api/hero-sms/sms/offer?country=6&service=tg&max_price_usd=0.6", "", "")
	if code != 200 {
		t.Fatal(out)
	}
	data := out["data"].(map[string]any)
	if data["bid"] != true || data["charge_quota"] != float64(300000) {
		t.Fatal(data)
	}
	code, out = webRequest(t, s, "GET", "/api/hero-sms/sms/orders/order1", "", "")
	raw, _ := json.Marshal(out)
	for _, forbidden := range []string{"supplier_cost_usd", "private-fingerprint", "private-proof", "cost_usd"} {
		if strings.Contains(string(raw), forbidden) {
			t.Fatalf("private supplier fact exposed: %s", forbidden)
		}
	}
	if code != 200 || out["data"].(map[string]any)["quota"] != float64(4350000) {
		t.Fatal(out)
	}
}

func TestHeroSMSPurchaseUsesStableCorePlanAndLiveBalance(t *testing.T) {
	var mu sync.Mutex
	var buyIDs []string
	calls := 0
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		var in map[string]any
		_ = json.NewDecoder(r.Body).Decode(&in)
		mu.Lock()
		defer mu.Unlock()
		calls++
		switch r.URL.Path {
		case "/internal/hero-sms/v1/plan":
			if in["request_key"] != "synthetic-request-key" || in["quantity"] != float64(1) || in["quote_id"] != "quote1" {
				t.Error(in)
			}
			writeJSON(w, map[string]any{"id": "batch1", "quote_id": "quote1", "order_ids": []string{"order1"}, "reorder_id": ""})
		case "/internal/hero-sms/v1/buy":
			buyIDs = append(buyIDs, in["id"].(string))
			if len(in) != 3 {
				t.Error("unexpected caller money fields", in)
			}
			writeJSON(w, syntheticReceipt())
		case "/internal/hero-sms/v1/balance":
			writeJSON(w, map[string]any{"quota": 4350000})
		default:
			t.Error(r.URL.Path)
		}
	})
	for i := 0; i < 2; i++ {
		code, out := webRequest(t, s, "POST", "/api/hero-sms/sms/orders", `{"offer_id":"quote1"}`, "synthetic-request-key")
		if code != 200 || out["data"].(map[string]any)["quota"] != float64(4350000) {
			t.Fatal(out)
		}
	}
	mu.Lock()
	if len(buyIDs) != 2 || buyIDs[0] != buyIDs[1] {
		t.Error("retry changed Core order identity", buyIDs)
	}
	before := calls
	mu.Unlock()
	for _, body := range []string{`{"offer_id":"quote1","charge_quota":1}`, `{"offer_id":"quote1","payer":42}`} {
		code, out := webRequest(t, s, "POST", "/api/hero-sms/sms/orders", body, "synthetic-request-key")
		if code != 400 {
			t.Fatal(out)
		}
	}
	mu.Lock()
	if calls != before {
		t.Error("invalid caller price/payer reached Core")
	}
	mu.Unlock()
	code, _ := webRequest(t, s, "POST", "/api/hero-sms/sms/orders", `{"offer_id":"quote1"}`, "")
	if code != 400 {
		t.Fatal("missing idempotency key accepted")
	}
}

func TestHeroSMSUnknownStaysReservedAndComplaintsDoNotClaimRefund(t *testing.T) {
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/internal/hero-sms/v1/poll":
			v := syntheticReceipt()
			v["provider_id"] = ""
			v["usable"] = false
			v["activation"] = map[string]any{"code": "must-not-leak"}
			writeJSON(w, v)
		case "/internal/hero-sms/v1/complaint":
			var in map[string]any
			_ = json.NewDecoder(r.Body).Decode(&in)
			if in["id"] != "order1" || in["reason"] != "SMS_NOT_RECEIVED" {
				t.Error(in)
			}
			v := syntheticReceipt()
			v["complaint_type"] = "SMS_NOT_RECEIVED"
			v["complaint_status"] = "submit_unknown"
			writeJSON(w, v)
		case "/internal/hero-sms/v1/balance":
			writeJSON(w, map[string]any{"quota": 4350000})
		default:
			t.Error(r.URL.Path)
		}
	})
	code, out := webRequest(t, s, "GET", "/api/hero-sms/sms/orders/order1", "", "")
	order := out["data"].(map[string]any)["order"].(map[string]any)
	if code != 200 || order["status"] != "purchase_unknown" || order["refunded_quota"] != float64(0) || order["can_cancel"] != false || order["code"] != "" {
		t.Fatal(out)
	}
	code, out = webRequest(t, s, "POST", "/api/hero-sms/sms/orders/order1/complaints", `{"reason":"SMS_NOT_RECEIVED"}`, "")
	order = out["data"].(map[string]any)["order"].(map[string]any)
	if code != 200 || order["complaint_status"] != "submit_unknown" || order["refunded_quota"] != float64(0) || order["can_complain"] != false {
		t.Fatal(out)
	}
}

func TestHeroSMSCoreFailuresAndCredentialAdministrationFailClosed(t *testing.T) {
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(503)
		_, _ = io.WriteString(w, `{"error":"UNKNOWN"}`)
	})
	code, out := webRequest(t, s, "GET", "/api/hero-sms/sms/offer?country=6&service=tg", "", "")
	if code != 503 || out["success"] != false {
		t.Fatal(out)
	}
	r := httptest.NewRequest("GET", "/api/option/hero-sms", nil)
	r.Header.Set("X-LMM-User-Credential", "other-session")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 403 {
		t.Fatal("Core permission rejection lost")
	}
	if _, err := NewCore("https://caller-chosen-host", []byte(syntheticServiceToken)); err == nil {
		t.Fatal("non-Unix Core transport accepted")
	}
}

func TestHeroSMSSettingsExposeOnlyPublicFileManagedConfiguration(t *testing.T) {
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/hero-sms/v1/settings" {
			t.Errorf("unexpected RPC %s", r.URL.Path)
		}
		writeJSON(w, map[string]any{
			"revision": int64(9007199254740993), "enabled": true, "sms_enabled": true,
			"email_enabled": false, "api_key_configured": true, "pending_work": true,
			"price_multiplier": "12.500000", "api_key": "synthetic-private-key",
			"credential_path": "/private/synthetic-key", "credential_sha256": "private-fingerprint",
		})
	})
	code, out := webRequest(t, s, "GET", "/api/option/hero-sms", "", "")
	data := out["data"].(map[string]any)
	if code != 200 || data["revision"] != "9007199254740993" || data["policy_managed_by_file"] != true || data["credential_managed_by_file"] != true || data["api_key_configured"] != true || data["sms_enabled"] != true || data["email_enabled"] != false || data["price_multiplier"] != "12.500000" {
		t.Fatal(out)
	}
	if data["currency"] != "USD" || data["currency_code"] != float64(840) || data["pending_work"] != true || data["purchases_available"] != false {
		t.Fatal(out)
	}
	for _, key := range []string{"api_key", "credential_path", "credential_sha256"} {
		if _, exists := data[key]; exists {
			t.Errorf("settings leaked %s", key)
		}
	}
}

func TestHeroSMSBrowserCannotChangeFileManagedPolicy(t *testing.T) {
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/hero-sms/v1/settings" {
			t.Errorf("unexpected RPC %s", r.URL.Path)
		}
		writeJSON(w, map[string]any{"revision": 4, "enabled": true, "price_multiplier": "10"})
	})
	for _, body := range []string{
		`{"expected_revision":"4","enabled":true,"sms_enabled":true,"email_enabled":false,"price_multiplier":"12"}`,
		`{"api_key":"caller-new-key"}`,
	} {
		code, out := webRequest(t, s, "PUT", "/api/option/hero-sms", body, "")
		if code != http.StatusConflict || out["code"] != "POLICY_MANAGED_BY_FILE" {
			t.Fatal(code, out)
		}
	}
}

func TestHeroSMSConnectionTestUsesOnlyTheConfiguredCredential(t *testing.T) {
	var calls atomic.Int32
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path != "/internal/hero-sms/v1/test" {
			t.Errorf("unexpected RPC %s", r.URL.Path)
		}
		var input map[string]any
		_ = json.NewDecoder(r.Body).Decode(&input)
		if len(input) != 0 {
			t.Error("connection test must not send browser credentials")
		}
		writeJSON(w, map[string]any{"ok": true, "api_key": "synthetic-private-key"})
	})
	code, out := webRequest(t, s, "POST", "/api/option/hero-sms/test", `{}`, "")
	data := out["data"].(map[string]any)
	if code != 200 || data["ok"] != true || len(data) != 1 || calls.Load() != 1 {
		t.Fatal(out)
	}
	code, out = webRequest(t, s, "POST", "/api/option/hero-sms/test", `{"api_key":"synthetic-candidate-key"}`, "")
	if code != 409 || out["code"] != "CREDENTIAL_MANAGED_BY_FILE" || calls.Load() != 1 {
		t.Fatal("candidate credentials must never reach Core", out)
	}
}

func TestHeroSMSPartialEmailBatchPreservesConfirmedActivations(t *testing.T) {
	s := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		var in map[string]any
		_ = json.NewDecoder(r.Body).Decode(&in)
		switch r.URL.Path {
		case "/internal/hero-sms/v1/plan":
			writeJSON(w, map[string]any{"id": "batch", "quote_id": "email-quote", "order_ids": []string{"email1", "email2"}, "reorder_id": ""})
		case "/internal/hero-sms/v1/buy":
			if in["id"] == "email2" {
				w.WriteHeader(503)
				_, _ = io.WriteString(w, `{"error":"UNKNOWN"}`)
				return
			}
			v := syntheticReceipt()
			v["id"] = "email1"
			v["charge_quota"] = 100000
			v["quote"] = map[string]any{"id": "email-quote", "selection": map[string]any{"kind": "email", "site": "example.test", "domain": "example.test"}}
			v["activation"] = map[string]any{"email": "example@example.test"}
			writeJSON(w, v)
		default:
			t.Error(r.URL.Path)
		}
	})
	code, out := webRequest(t, s, "POST", "/api/hero-sms/email/activations", `{"domain_id":"email-quote","quantity":2}`, "synthetic-email-batch-key")
	if code != 200 {
		t.Fatal(out)
	}
	data := out["data"].(map[string]any)
	order := data["order"].(map[string]any)
	if order["status"] != "partial" || order["charge_quota"] != float64(100000) || order["quantity"] != float64(2) || len(data["activations"].([]any)) != 1 {
		t.Fatal(out)
	}
}

func TestHeroSMSPreservesSessionAndProviderCredentialFailures(t *testing.T) {
	for _, tc := range []struct {
		status int
		code   string
	}{
		{401, "UNAUTHORIZED"},
		{502, "PROVIDER_CREDENTIAL_REJECTED"},
	} {
		t.Run(tc.code, func(t *testing.T) {
			service := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(tc.status)
				writeJSON(w, map[string]any{"error": tc.code})
			})
			status, out := webRequest(t, service, "POST", "/api/option/hero-sms/test", `{}`, "")
			if status != tc.status || out["code"] != tc.code {
				t.Fatal(status, out)
			}
		})
	}
}

func TestHeroSMSPurchaseCapabilitiesAreExplicitAndDoNotExposeSettings(t *testing.T) {
	for _, tc := range []struct {
		name  string
		field any
		want  bool
	}{
		{"missing", nil, false},
		{"not implemented", false, false},
		{"implemented", true, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			service := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/internal/hero-sms/v1/ready" || r.Header.Get("X-LMM-User-Credential") != "" {
					t.Error("capability discovery must use only the separate host authority")
				}
				out := map[string]any{"protocol": 1, "service": "hero-sms", "ready": true, "api_key": "synthetic-private-key", "price_multiplier": "10"}
				if tc.field != nil {
					out["purchases_available"] = tc.field
				}
				writeJSON(w, out)
			})
			code, out := webRequest(t, service, "GET", "/api/hero-sms/capabilities", "", "")
			data, ok := out["data"].(map[string]any)
			if code != http.StatusOK || !ok || len(data) != 1 || data["purchases_available"] != tc.want {
				t.Fatal(code, out)
			}
		})
	}
	service := serviceFixture(t, func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
		writeJSON(w, map[string]any{"error": "HERO_SMS_UNAVAILABLE"})
	})
	code, out := webRequest(t, service, "GET", "/api/hero-sms/capabilities", "", "")
	if code != http.StatusServiceUnavailable || out["success"] != false || out["data"] != nil {
		t.Fatal("a failed capability read must not enable purchasing", code, out)
	}
	request := httptest.NewRequest(http.MethodGet, "/api/hero-sms/capabilities", nil)
	response := httptest.NewRecorder()
	service.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden {
		t.Fatal("capability route requires an authenticated user")
	}
}
