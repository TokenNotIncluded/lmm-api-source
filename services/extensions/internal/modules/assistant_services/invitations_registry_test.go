package assistant_services

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
)

func TestInvitationInstalledRegistryMatchesActualBindings(t *testing.T) {
	raw, e := os.ReadFile("../../../../../contracts/assistant-tools-v2.json")
	if e != nil {
		t.Fatal(e)
	}
	var file struct {
		Tools []map[string]string `json:"tools"`
	}
	if json.Unmarshal(raw, &file) != nil {
		t.Fatal("invalid installed registry")
	}
	found := 0
	for _, b := range (&Service{}).Bindings() {
		if b.Name != "get_invitation_rewards" && b.Name != "send_invitation" {
			continue
		}
		h := sha256.Sum256(b.Parameters)
		wanted := map[string]string{"name": b.Name, "domain": "extension", "description": b.Description, "version": b.Version, "schema_json": string(b.Parameters), "schema_sha256": hex.EncodeToString(h[:]), "price_version": b.PriceVersion, "price_sha256": b.PriceSHA256}
		count := 0
		for _, row := range file.Tools {
			if row["name"] == b.Name {
				count++
				if len(row) != len(wanted) {
					t.Fatal("unexpected invitation descriptor fields")
				}
				for k, v := range wanted {
					if row[k] != v {
						t.Fatalf("invitation descriptor mismatch: %s %s", b.Name, k)
					}
				}
			}
		}
		if count != 1 {
			t.Fatal("invitation descriptor not unique")
		}
		found++
	}
	if found != 2 {
		t.Fatal("missing invitation bindings")
	}
}
