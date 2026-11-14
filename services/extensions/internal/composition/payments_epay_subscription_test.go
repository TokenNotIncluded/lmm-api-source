package composition

import (
	"fmt"
	"testing"
)

func TestEPaySubscriptionAdmissionIndependentOfWallet(t *testing.T) {
	for _, wallet := range []bool{false, true} {
		for _, subscription := range []bool{false, true} {
			raw := fmt.Sprintf(`version=2
[[channels]]
id=2
provider='epay'
merchant='1002'
environment='live'
currency='LDC'
enabled=%t
subscription_enabled=%t
payment_type='epay'
`, wallet, subscription)
			admission := map[int64]bool{}
			adapters, err := loadMultiPaymentAdaptersWithAdmission([]byte(raw), coreCheckoutSigner{}, admission)
			if err != nil || len(adapters) != 1 || adapters[0].Channel().Enabled != wallet || admission[2] != subscription {
				t.Fatal("EPay wallet and subscription admission coupled", err)
			}
		}
	}
}
