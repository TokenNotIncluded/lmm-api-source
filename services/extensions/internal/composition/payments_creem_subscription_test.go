package composition

import (
	"fmt"
	"net/http/httptest"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

func TestCreemSubscriptionAdmissionIndependentOfWalletProduct(t *testing.T) {
	key := paymentPrivate(t, "creem-subscription-key", "creem_synthetic_subscription_key_0001")
	for _, currency := range []string{"USD", "CNY", "EUR"} {
		for _, enabled := range []bool{false, true} {
			raw := fmt.Sprintf(`version=2
[[channels]]
id=9
provider='creem'
merchant='creem_subscription_fixture'
environment='test'
currency='%s'
enabled=false
subscription_enabled=%t
api_key_file='%s'
success_url='https://example.test/paid'
`, currency, enabled, key)
			admission := map[int64]bool{}
			adapters, err := loadMultiPaymentAdaptersWithAdmission([]byte(raw), nil, admission)
			if err != nil || len(adapters) != 1 || adapters[0].Channel().Enabled || admission[9] != enabled {
				t.Fatal("subscription-only Creem configuration rejected or coupled to wallet", err)
			}
			if _, ok := adapters[0].(*payments.Creem); !ok {
				t.Fatal("Creem adapter not assembled")
			}
		}
	}
}

func TestCreemSubscriptionOnlyRuntimeMountsUserRoutes(t *testing.T) {
	r := &Runtime{CreemSubscriptionPayments: &payments.CreemSubscriptions{}}
	h := r.SubscriptionPaymentHandler()
	if h == nil {
		t.Fatal("Creem-only subscriptions disappeared")
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest("GET", "/creem/orders/original", nil))
	if w.Code != 403 {
		t.Fatal("subscription route absent or missing credential admitted", w.Code)
	}
}
