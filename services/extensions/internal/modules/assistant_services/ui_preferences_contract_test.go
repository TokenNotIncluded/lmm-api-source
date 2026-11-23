package assistant_services

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

func TestUIPreferenceAndCostRegistryMatchesActualAvailableBindings(t *testing.T) {
	raw, err := os.ReadFile("../../../../../contracts/assistant-tools-v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var registry struct {
		Tools []map[string]any `json:"tools"`
	}
	if json.Unmarshal(raw, &registry) != nil {
		t.Fatal("invalid registry")
	}
	rows := map[string]map[string]any{}
	for _, row := range registry.Tools {
		name, ok := row["name"].(string)
		if !ok || rows[name] != nil {
			t.Fatal("duplicate/missing tool")
		}
		rows[name] = row
	}
	selected := map[string]bool{"calculate_cost": true, "get_ui_preference_options": true, "set_ui_preferences": true, "restore_ui_preferences": true}
	for _, binding := range (&Service{}).AvailableBindings() {
		if !selected[binding.Name] {
			continue
		}
		delete(selected, binding.Name)
		h := sha256.Sum256(binding.Parameters)
		expected := map[string]any{"name": binding.Name, "description": binding.Description, "version": binding.Version, "domain": "extension", "schema_json": string(binding.Parameters), "schema_sha256": hex.EncodeToString(h[:]), "price_version": binding.PriceVersion, "price_sha256": binding.PriceSHA256}
		if !reflect.DeepEqual(rows[binding.Name], expected) {
			t.Fatalf("installed definition differs from actual executor: %s", binding.Name)
		}
		if binding.Access != "user" || (binding.Name == "calculate_cost" || binding.Name == "get_ui_preference_options") && binding.Effect != "read_only" || (binding.Name == "set_ui_preferences" || binding.Name == "restore_ui_preferences") && binding.Effect != "server_guarded" {
			t.Fatal("display/estimate authority changed")
		}
	}
	if len(selected) != 0 {
		t.Fatal("advertised tool lacks actual binding")
	}
}
