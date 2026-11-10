package freshinstall

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

func TestPaymentsFreshPlanIncludesStripeWithoutReplacingWalletOrEPay(t *testing.T) {
	p, err := compiled("payments")
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"payment_storage_guard", "payment_orders", "payment_events", "payment_subscription_inbox", "payment_stripe_subscription_checkouts", "payment_stripe_subscription_inbox"} {
		if !strings.Contains(p.schema, "CREATE TABLE "+name) {
			t.Fatal("missing installed table", name)
		}
		found := false
		for _, g := range p.grants {
			if strings.Contains(g, name) {
				found = true
				if strings.Contains(g, "DELETE") || strings.Contains(g, "TRUNCATE") || strings.Contains(g, "ALL ") {
					t.Fatal("excessive runtime privileges", name)
				}
			}
		}
		if !found {
			t.Fatal("missing runtime grant", name)
		}
	}
}
func TestStripeOfflineCLIRejectsUnreviewedInputs(t *testing.T) {
	for _, args := range [][]string{{}, {"--sql-file", "/tmp/source.sql"}, {"--database", "lmm_payments_test", "--recover", "extra"}, {"--config", "runtime.toml"}} {
		var out bytes.Buffer
		if RunPaymentsStripeSubscriptionUpgradeCLI(context.Background(), args, &out) == nil || out.Len() != 0 {
			t.Fatal("invalid Stripe offline operation accepted")
		}
	}
}
