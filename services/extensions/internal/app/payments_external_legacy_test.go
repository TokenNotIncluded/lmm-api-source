package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPaymentsExternalLegacyPancakeOwners(t *testing.T) {
	const base = "version=1\n[host]\ntoken_file='/tmp/host-token'\nmodules=['payments']\n[core]\nsocket='/tmp/core.sock'\nidentity_token_file='/tmp/identity-token'\npayment_token_file='/tmp/payment-token'\n[payments]\ndatabase_name='lmm_payments_test'\ndatabase_url_file='/tmp/payment.url'\nconfig_file='/tmp/payment.toml'\ncallbacks_enabled=true\n"
	const shared = "\n[[payments.external_legacy_pancake_owners]]\nstore_id='shared_store'\nenvironment='live'\n"
	const exclusive = shared + "native_product_ids=['native_usd','native_cny']\n"
	write := func(raw string) string {
		t.Helper()
		path := filepath.Join(t.TempDir(), "extensions.toml")
		if err := os.WriteFile(path, []byte(raw), 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	for _, tc := range []struct {
		name, suffix     string
		owners, products int
	}{
		{"default_absent", "", 0, 0},
		{"default_empty", "external_legacy_pancake_owners=[]\n", 0, 0},
		{"shared_sku_omitted_list", shared, 1, 0},
		{"shared_sku_empty_list", shared + "native_product_ids=[]\n", 1, 0},
		{"exclusive_sku_list", exclusive, 1, 2},
		{"separate_environments", shared + strings.Replace(shared, "'live'", "'test'", 1), 2, 0},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cfg, err := LoadConfig(write(base + tc.suffix))
			if err != nil {
				t.Fatal(err)
			}
			owners := cfg.Payments.ExternalLegacyPancakeOwners
			if len(owners) != tc.owners {
				t.Fatalf("owner count %d, want %d", len(owners), tc.owners)
			}
			if tc.owners != 0 && (owners[0].StoreID != "shared_store" || owners[0].Environment != "live" ||
				len(owners[0].NativeProductIDs) != tc.products) {
				t.Fatal("explicit ownership fields changed")
			}
		})
	}
	for _, tc := range []struct{ name, suffix string }{
		{"missing_store", strings.Replace(shared, "store_id='shared_store'\n", "", 1)},
		{"store_whitespace", strings.Replace(shared, "'shared_store'", "' shared_store'", 1)},
		{"missing_environment", strings.Replace(shared, "environment='live'\n", "", 1)},
		{"unknown_environment", strings.Replace(shared, "'live'", "'production'", 1)},
		{"wire_mode_is_not_config_environment", strings.Replace(shared, "'live'", "'prod'", 1)},
		{"duplicate_store_environment", shared + shared},
		{"invalid_product", shared + "native_product_ids=['']\n"},
		{"duplicate_product", shared + "native_product_ids=['native','native']\n"},
		{"noncanonical_key", strings.Replace(shared, "store_id=", "StoreID=", 1)},
		{"unreviewed_public_key", shared + "public_key='untrusted'\n"},
		{"unreviewed_namespace", shared + "namespace='top_'\n"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := LoadConfig(write(base + tc.suffix)); err == nil {
				t.Fatal("invalid external legacy ownership configuration accepted")
			}
		})
	}
	t.Run("requires_payment_module", func(t *testing.T) {
		raw := "version=1\n[host]\ntoken_file='/tmp/host-token'\nmodules=['identity']\n[core]\nsocket='/tmp/core.sock'\nidentity_token_file='/tmp/identity-token'\n" + shared
		if _, err := LoadConfig(write(raw)); err == nil {
			t.Fatal("ownership rule enabled without payment module")
		}
	})
}
