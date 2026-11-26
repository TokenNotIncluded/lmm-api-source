// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package payments

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestProviderGatewayConfigurationProjectsLoadedBindings(t *testing.T) {
	const secret = "synthetic-credential-never-return"
	stripe, err := NewStripe(StripeConfig{
		Channel: Channel{ID: 20, Provider: "stripe", Merchant: "acct_test", Environment: "test", Currency: "USD"},
		APIKey:  "sk_test_" + secret, APIVersion: "2025-01-01",
		SuccessURL: "https://example.test/paid", CancelURL: "https://example.test/cancelled",
	})
	if err != nil {
		t.Fatal(err)
	}
	coinbase := coinbaseFixture(t)
	configuration := DescribeProviderGateways([]Adapter{stripe, coinbase}, map[int64]bool{20: true}, true)
	if len(configuration.Bindings) != 2 || !configuration.CallbacksEnabled {
		t.Fatalf("unexpected configuration: %+v", configuration)
	}
	for _, binding := range configuration.Bindings {
		switch binding.Provider {
		case "stripe":
			if binding.WalletEnabled || binding.SubscriptionEnabled == nil || !*binding.SubscriptionEnabled ||
				!binding.APIKeyConfigured || binding.APIVersion != "2025-01-01" || binding.SuccessURL != "https://example.test/paid" {
				t.Fatalf("unexpected Stripe policy: %+v", binding)
			}
		case "coinbase":
			if binding.PrivateKeyConfigured == nil || !*binding.PrivateKeyConfigured || binding.SubscriptionEnabled != nil {
				t.Fatalf("unexpected Coinbase policy: %+v", binding)
			}
		default:
			t.Fatalf("unexpected provider: %q", binding.Provider)
		}
	}
	raw, err := json.Marshal(configuration)
	if err != nil || strings.Contains(string(raw), secret) || strings.Contains(string(raw), coinbase.cfg.APIKey) {
		t.Fatal("provider configuration must never serialize API credentials")
	}
}
