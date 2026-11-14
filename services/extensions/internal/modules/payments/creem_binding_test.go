package payments

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"testing"
	"time"
)

// Core accepts both reference forms. The observation adapter must not discard
// an expanded product before the original webhook reaches Core's proof gate.
func TestCreemOrderProductReferencesInCallbackAndLookup(t *testing.T) {
	for _, tc := range []struct {
		name, product string
		conflict      bool
	}{
		{"id", `"prod_fixture"`, false},
		{"expanded", `{"id":"prod_fixture","object":"product"}`, false},
		{"wrong_id", `"prod_other"`, true},
		{"wrong_expanded", `{"id":"prod_other","object":"product"}`, true},
		{"empty_expanded", `{}`, true},
		{"null", `null`, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			c := creemFixture(t)
			raw := strings.Replace(creemPaid, `"transaction":"tran_fixture"`, `"product":`+tc.product+`,"transaction":"tran_fixture"`, 1)
			var envelope struct {
				Object json.RawMessage `json:"object"`
			}
			if err := json.Unmarshal([]byte(raw), &envelope); err != nil {
				t.Fatal(err)
			}
			c.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
				if r.Method != http.MethodGet || r.URL.Path != "/v1/checkouts" || r.URL.Query().Get("checkout_id") != "ch_fixture" {
					t.Fatal("lookup changed checkout identity", r.URL)
				}
				return response(string(envelope.Object)), nil
			})}
			o := gatewayOrder(c.Channel())
			o.Checkout.ID = "ch_fixture"
			callback, callbackErr := c.Verify(gatewayCallback(raw, "Creem-Signature"), time.Now())
			events, lookupErr := c.Lookup(context.Background(), o)
			if tc.conflict {
				if !errors.Is(callbackErr, ErrConflict) || !errors.Is(lookupErr, ErrConflict) || len(events) != 0 {
					t.Fatal("invalid product accepted", callbackErr, lookupErr, events)
				}
				return
			}
			if callbackErr != nil || lookupErr != nil || len(events) != 1 {
				t.Fatal("valid product rejected", callbackErr, lookupErr, events)
			}
			if string(callback.Evidence.Payload) != raw || len(callback.Evidence.Signature) == 0 || callback.Evidence.AcceptedProofID != "" || callback.OrderID != o.ID || callback.AmountMinor != o.Intent.AmountMinor {
				t.Fatal("callback lost original evidence or invented proof", callback)
			}
			if string(events[0].Evidence.Payload) != string(envelope.Object) || events[0].Evidence.Source != "api" || len(events[0].Evidence.Signature) != 0 || events[0].Evidence.AcceptedProofID != "" {
				t.Fatal("query lost observation or invented proof", events)
			}
		})
	}
}
