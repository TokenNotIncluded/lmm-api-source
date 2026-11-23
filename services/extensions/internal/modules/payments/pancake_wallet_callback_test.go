package payments

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// This checks routing and durable-proof handoff with the existing memory/Core
// fixtures; it does not certify provider signatures or PostgreSQL durability.
func TestPancakeOriginalWebhookRoutesFrozenNativeOrder(t *testing.T) {
	c := Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD", Enabled: false}
	p := &Pancake{cfg: PancakeConfig{Channel: c, StoreID: "replacement_store", ProductID: "replacement_product"}}
	for _, tc := range []struct {
		name, payload string
		status        int
	}{
		{"original", pancakePaid, http.StatusOK},
		{"other_store", strings.Replace(pancakePaid, "STO_0123456789abcdefghijkl", "replacement_store", 1), http.StatusConflict},
		{"other_currency", strings.Replace(pancakePaid, `"currency":"USD"`, `"currency":"CNY"`, 1), http.StatusConflict},
		{"other_environment", strings.Replace(pancakePaid, `"mode":"test"`, `"mode":"prod"`, 1), http.StatusBadRequest},
		{"missing_order", strings.Replace(pancakePaid, "top_1", "top_2", 1), http.StatusNotFound},
		{"malformed_native", strings.Replace(pancakePaid, "top_1", "top_01", 1), http.StatusBadRequest},
		{"duplicate_reference", strings.Replace(pancakePaid, `"orderMerchantExternalId":"top_1"`, `"orderMerchantExternalId":"top_2","orderMerchantExternalId":"top_1"`, 1), http.StatusBadRequest},
	} {
		t.Run(tc.name, func(t *testing.T) {
			store := newMemory()
			if err := store.Insert(context.Background(), gatewayOrder(c)); err != nil {
				t.Fatal(err)
			}
			core := newCore(c)
			wallet, err := New(store, core, p)
			if err != nil {
				t.Fatal(err)
			}
			subscriptions := &PancakeSubscriptions{adapters: map[int64]*Pancake{c.ID: p}}
			for attempt := 0; attempt < 2; attempt++ {
				// Reconstruct the service to exercise the persisted order binding.
				wallet, err = New(store, core, p)
				if err != nil {
					t.Fatal(err)
				}
				mux := http.NewServeMux()
				mux.Handle("POST /api/waffo-pancake/webhook/{environment}", subscriptions.LegacyWebhookHandler(wallet, ExternalLegacyPancakeOwner{
					StoreID: "STO_0123456789abcdefghijkl", Environment: "test",
					NativeProductIDs: []string{"PROD_0123456789abcdefghijkl", "replacement_product"},
				}))
				req := httptest.NewRequest(http.MethodPost, "/api/waffo-pancake/webhook/test", strings.NewReader(tc.payload))
				req.Header.Set("X-Waffo-Signature", "original-provider-signature")
				response := httptest.NewRecorder()
				mux.ServeHTTP(response, req)
				if response.Code != tc.status {
					t.Fatalf("status %d, want %d: %s", response.Code, tc.status, response.Body.String())
				}
			}
			if tc.status == http.StatusOK {
				if len(store.events) != 1 || core.count("accept_proof") != 1 {
					t.Fatal("replay did not retain one accepted event")
				}
			} else if len(store.events) != 0 || core.count("accept_proof") != 0 {
				t.Fatal("invalid callback reached Core or inbox")
			}
			if core.count("credit") != 0 {
				t.Fatal("HTTP acknowledgement claimed wallet credit")
			}
		})
	}
}
