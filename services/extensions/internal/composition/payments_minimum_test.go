package composition

import (
	"fmt"
	"testing"
)

func TestPaymentV2MinimumWalletAmountDefaultsAndPreservesExplicitFloor(t *testing.T) {
	for _, tc := range []struct {
		name, field string
		want        int64
		invalid     bool
	}{
		{"omitted", "", 1, false},
		{"legacy-ldc", "min_amount_minor=10000", 10000, false},
		{"maximum", "min_amount_minor=9000000000000", 9000000000000, false},
		{"zero", "min_amount_minor=0", 0, true},
		{"negative", "min_amount_minor=-1", 0, true},
		{"over-native-limit", "min_amount_minor=9000000000001", 0, true},
		{"non-integer", "min_amount_minor='10000'", 0, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			raw := fmt.Sprintf(`version=2
[[channels]]
id=5004
provider='epay'
merchant='123'
environment='live'
currency='LDC'
payment_type='epay'
enabled=false
subscription_enabled=true
%s
`, tc.field)
			admission := map[int64]bool{}
			adapters, err := loadMultiPaymentAdaptersWithAdmission([]byte(raw), coreCheckoutSigner{}, admission)
			if tc.invalid {
				if err == nil {
					t.Fatal("invalid wallet minimum accepted")
				}
				return
			}
			if err != nil || len(adapters) != 1 {
				t.Fatal("valid minimum rejected", err)
			}
			channel := adapters[0].Channel()
			if channel.MinAmountMinor != tc.want || channel.Enabled || !admission[5004] {
				t.Fatal("minimum or independent subscription admission changed", channel.MinAmountMinor)
			}
		})
	}
}
