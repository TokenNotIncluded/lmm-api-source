package composition

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"reflect"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/personalization"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_market"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func TestReviewedRestoredToolsMatchRealDomainBindings(t *testing.T) {
	raw, err := os.ReadFile("../../../../contracts/assistant-tools-v2.json")
	if err != nil {
		t.Fatal(err)
	}
	if len(raw) < 22849 {
		t.Fatal("reviewed 23-tool prefix truncated")
	}
	prefix := sha256.Sum256(raw[:22849])
	if hex.EncodeToString(prefix[:]) != "b334ea62c29c44d9dda971ccbb03b748ae6b69da7a95acb6602bd62b8ab9b034" {
		t.Fatal("prior reviewed 23 tools changed")
	}
	var artifact struct {
		Tools []map[string]any `json:"tools"`
	}
	if json.Unmarshal(raw, &artifact) != nil || len(artifact.Tools) != 61 {
		t.Fatal("unexpected reviewed scope")
	}
	rows := map[string]map[string]any{}
	for _, row := range artifact.Tools {
		name := row["name"].(string)
		if rows[name] != nil {
			t.Fatal("duplicate tool")
		}
		rows[name] = row
	}
	selected := map[string]bool{"get_service_facts": true, "get_setup_guide": true, "navigate_to_page": true, "get_site_policy": true, "search_site_policies": true, "get_human_support_status": true, "book_technical_support": true, "request_human_support": true, "create_site_issue": true, "update_site_issue": true, "set_overview_greeting": true, "get_tool_market_services": true, "get_tool_market_service": true, "get_connected_market_tools": true, "call_market_tool": true}
	bindings := append((&assistant_services.Service{}).Bindings(), (&personalization.Service{}).Bindings()...)
	market, err := assistant_market.New(&toolmarket.Module{})
	if err != nil {
		t.Fatal(err)
	}
	more, err := market.Bindings()
	if err != nil {
		t.Fatal(err)
	}
	bindings = append(bindings, more...)
	var included []assistant.AgentToolBinding
	for _, b := range bindings {
		if !selected[b.Name] {
			continue
		}
		delete(selected, b.Name)
		included = append(included, b)
		h := sha256.Sum256(b.Parameters)
		expected := map[string]any{"name": b.Name, "description": b.Description, "version": b.Version, "domain": "extension", "schema_json": string(b.Parameters), "schema_sha256": hex.EncodeToString(h[:]), "price_version": b.PriceVersion, "price_sha256": b.PriceSHA256}
		if !reflect.DeepEqual(rows[b.Name], expected) {
			t.Fatalf("reviewed descriptor differs from real binding: %s", b.Name)
		}
	}
	if len(selected) != 0 || len(included) != 15 {
		t.Fatal("missing real executor", selected)
	}
	if _, err = assistant.NewAgentRegistry(included); err != nil {
		t.Fatal(err)
	}
	// Unimplemented administrative authority stays absent. Browser-confirmed
	// account actions are Core-owned and must not enter extension dispatch.
	for _, name := range []string{"prepare_user_action", "prepare_api_key_action", "request_create_key", "connect_market_tool"} {
		if rows[name] == nil || rows[name]["domain"] != "registered_core" {
			t.Fatalf("browser-confirmed action must remain Core-owned: %s", name)
		}
		for _, binding := range bindings {
			if binding.Name == name {
				t.Fatalf("Core action exposed as extension: %s", name)
			}
		}
	}
	for _, name := range []string{"grant_l1_access", "prepare_admin_site_policy_change"} {
		if rows[name] != nil {
			t.Fatalf("unreviewed authority added: %s", name)
		}
	}
}

func TestReviewedTypedReadRegistryMatchesCoreManifest(t *testing.T) {
	read := func(path string) []map[string]any {
		t.Helper()
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var rows []map[string]any
		if json.Unmarshal(raw, &rows) != nil {
			t.Fatal("invalid typed manifest")
		}
		return rows
	}
	manifest := read("../../../core/src/assistant_tools_core/manifest.json")
	raw, err := os.ReadFile("../../../../contracts/assistant-tools-v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var artifact struct {
		Tools []map[string]any `json:"tools"`
	}
	if json.Unmarshal(raw, &artifact) != nil {
		t.Fatal("invalid installed registry")
	}
	for _, name := range []string{"get_user_usage_summary", "get_usage_summary", "get_weekly_discount_status"} {
		var core, installed map[string]any
		for _, row := range manifest {
			if row["name"] == name {
				core = row
			}
		}
		for _, row := range artifact.Tools {
			if row["name"] == name {
				installed = row
			}
		}
		if core == nil || installed == nil {
			t.Fatal("missing usage read", name)
		}
		schema, err := json.Marshal(core["parameters"])
		if err != nil {
			t.Fatal(err)
		}
		hash := sha256.Sum256(schema)
		if core["schema_sha256"] != hex.EncodeToString(hash[:]) || installed["schema_json"] != string(schema) || installed["schema_sha256"] != core["schema_sha256"] {
			t.Fatal("typed read schema differs", name)
		}
		for _, key := range []string{"name", "description", "version", "domain", "price_version", "price_sha256"} {
			if installed[key] != core[key] {
				t.Fatal("typed read descriptor mismatch", name, key)
			}
		}
	}
}
