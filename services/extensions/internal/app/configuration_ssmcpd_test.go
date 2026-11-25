package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLoadSSMCPDConfig(t *testing.T) {
	const fixture = `version=1
[host]
token_file="/tmp/host-fixture"
modules=["ssmcpd"]
[core]
socket="/tmp/core-fixture.sock"
identity_token_file="/tmp/core-fixture-token"
[ssmcpd]
mode="relay"
public_origin="https://api.lmm.best"
traffic_portal=false
[ssmcpd.metering]
origin="http://127.0.0.1:12345"
token_file="/tmp/meter-fixture"
[ssmcpd.commerce]
shop_id="shop-fixture"
variant_id="variant-fixture"
bytes_per_unit=1000
[ssmcpd.commerce.grants]
origin="http://127.0.0.1:12345"
token_file="/tmp/grants-fixture"
`
	path := filepath.Join(t.TempDir(), "host.toml")
	for _, key := range []string{"bytes_per_unit", "Bytes_Per_Unit", "bytes_per_unitt"} {
		if err := os.WriteFile(path, []byte(strings.Replace(fixture, "bytes_per_unit", key, 1)), 0600); err != nil {
			t.Fatal(err)
		}
		cfg, err := LoadConfig(path)
		if key != "bytes_per_unit" {
			if err == nil {
				t.Fatalf("accepted %s", key)
			}
			continue
		}
		if err != nil {
			t.Fatal(err)
		}
		s := cfg.SSMCPD
		if s == nil || s.Mode != "relay" || s.Metering == nil || s.Metering.Origin != "http://127.0.0.1:12345" || s.Commerce == nil || s.Commerce.BytesPerUnit != 1000 || s.Commerce.Grants.TokenFile != "/tmp/grants-fixture" {
			t.Fatal("incomplete ssmcpd configuration")
		}
	}
}
