package composition

import (
	"context"
	"fmt"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

func TestStripeSubscriptionAdmissionIndependentOfWallet(t *testing.T) {
	key := paymentPrivate(t, "stripe-subscription-key", "sk_test_synthetic_subscription_api_0001")
	for _, v := range []struct{ wallet, subscription bool }{{true, false}, {false, true}, {false, false}, {true, true}} {
		raw := fmt.Sprintf(`version=2
[[channels]]
id=8
provider='stripe'
merchant='acct_subscription_fixture'
environment='test'
currency='USD'
enabled=%t
subscription_enabled=%t
api_key_file='%s'
api_version='2025-02-24.acacia'
success_url='https://example.test/paid'
cancel_url='https://example.test/cancel'
`, v.wallet, v.subscription, key)
		admission := map[int64]bool{}
		adapters, err := loadMultiPaymentAdaptersWithAdmission([]byte(raw), nil, admission)
		if err != nil || len(adapters) != 1 {
			t.Fatal(err)
		}
		if adapters[0].Channel().Enabled != v.wallet || admission[8] != v.subscription {
			t.Fatal("wallet and subscription admission coupled")
		}
		if v.subscription {
			raw = strings.Replace(raw, "subscription_enabled=true\n", "", 1)
			admission = map[int64]bool{}
			_, err = loadMultiPaymentAdaptersWithAdmission([]byte(raw), nil, admission)
			if err != nil || admission[8] {
				t.Fatal("omitted SUB admission enabled buying", err)
			}
		}
	}
	// Version1 uses the same document read and does not inherit wallet enabled.
	raw := fmt.Sprintf(`version=1
provider='stripe-test'
[[channels]]
id=8
merchant='acct_subscription_fixture'
enabled=false
subscription_enabled=true
api_key_file='%s'
api_version='2025-02-24.acacia'
success_url='https://example.test/paid'
cancel_url='https://example.test/cancel'
`, key)
	path := paymentPrivate(t, "stripe-subscription-config", raw)
	admission := map[int64]bool{}
	adapters, err := loadPaymentAdaptersWithAdmission(context.Background(), path, "", false, nil, admission)
	if err != nil || len(adapters) != 1 || adapters[0].Channel().Enabled || !admission[8] {
		t.Fatal("version1 admission mismatch", err)
	}
}

func TestStripeSubscriptionOnlyRuntimeMountsUserRoutes(t *testing.T) {
	r := &Runtime{StripeSubscriptionPayments: &payments.StripeSubscriptions{}}
	h := r.SubscriptionPaymentHandler()
	if h == nil {
		t.Fatal("Stripe-only subscriptions disappeared")
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest("GET", "/stripe/orders/original", nil))
	if w.Code == 404 || w.Code == 200 {
		t.Fatal("subscription route absent or missing credential admitted", w.Code)
	}
	if (&Runtime{}).SubscriptionPaymentHandler() != nil {
		t.Fatal("empty runtime exposed subscriptions")
	}
}
