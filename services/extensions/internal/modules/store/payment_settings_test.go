package store

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestPaymentSettingsReflectMountedCommerceWithoutWrites(t *testing.T) {
	for _, available := range []bool{false, true} {
		name := "disabled"
		var funds Funds
		probe := newFunds()
		if available {
			name, funds = "negotiated", probe
		}
		t.Run(name, func(t *testing.T) {
			repo := newRepo()
			auth := &authority{}
			service, err := New(repo, auth, funds)
			must(t, err)
			value, err := service.Legacy(context.Background(), sellerToken, Account{"personal", 1}, LegacyRequest{Action: "payment_settings"})
			must(t, err)
			view := value.(map[string]any)
			if view["read_only"] != true || view["external_eligible"] != false {
				t.Fatal(view)
			}
			methods := view["items"].([]map[string]any)
			if len(methods) != 1 || methods[0]["provider"] != "balance" || methods[0]["effective_enabled"] != available || methods[0]["configured"] != available {
				t.Fatal(view)
			}
			categories := view["categories"].(map[string]bool)
			if categories["platform_enabled"] != available || categories["external_enabled"] {
				t.Fatal(categories)
			}
			for _, field := range []string{"balance_quota", "fee_bps"} {
				if _, invented := view[field]; invented {
					t.Fatalf("settings fabricated %s without a Core read", field)
				}
			}
			raw, err := json.Marshal(view)
			must(t, err)
			if strings.Contains(string(raw), sellerToken) || len(repo.data) != 0 || len(probe.commands) != 0 {
				t.Fatal("capability read disclosed credentials or mutated state")
			}
			if _, err = service.Legacy(context.Background(), sellerToken, Account{"personal", 2}, LegacyRequest{Action: "payment_settings"}); !errors.Is(err, ErrForbidden) {
				t.Fatal("cross-account read", err)
			}
			auth.revoked.Store(true)
			if _, err = service.Legacy(context.Background(), sellerToken, Account{"personal", 1}, LegacyRequest{Action: "payment_settings"}); !errors.Is(err, ErrForbidden) {
				t.Fatal("revoked session", err)
			}
		})
	}
}
