package assistant_services

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

func TestPlanOffersRegistryMatchesActualAvailableBinding(t *testing.T) {
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
			t.Fatal("duplicate or missing tool name")
		}
		rows[name] = row
	}
	reader, err := NewCorePlanOffers(&planOffersPort{facts: offersFixture()}, &planOffersAuthority{}, offersCatalogue{})
	if err != nil {
		t.Fatal(err)
	}
	service := &Service{dependencies: Dependencies{PlanOffers: reader}}
	matches := 0
	for _, binding := range service.AvailableBindings() {
		if binding.Name != "get_plan_offers" {
			continue
		}
		matches++
		h := sha256.Sum256(binding.Parameters)
		expected := map[string]any{
			"name": binding.Name, "description": binding.Description, "version": binding.Version,
			"domain": "extension", "schema_json": string(binding.Parameters), "schema_sha256": hex.EncodeToString(h[:]),
			"price_version": binding.PriceVersion, "price_sha256": binding.PriceSHA256,
		}
		if !reflect.DeepEqual(rows[binding.Name], expected) {
			t.Fatal("installed plan offer definition differs from executable binding")
		}
		if binding.Access != "user" || binding.Effect != "read_only" {
			t.Fatal("plan offer read authority changed")
		}
	}
	if matches != 1 {
		t.Fatal("configured reader not uniquely advertised")
	}
}
