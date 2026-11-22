package assistant_services

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"reflect"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"testing"
)

func TestReviewedSearchRegistryMatchesAvailableBinding(t *testing.T) {
	read := func(name string) map[string]map[string]any {
		t.Helper()
		raw, err := os.ReadFile("../../../../../contracts/" + name)
		if err != nil {
			t.Fatal(err)
		}
		var artifact struct {
			Tools []map[string]any `json:"tools"`
		}
		if json.Unmarshal(raw, &artifact) != nil {
			t.Fatal("invalid reviewed registry")
		}
		out := map[string]map[string]any{}
		for _, row := range artifact.Tools {
			name, ok := row["name"].(string)
			if !ok || out[name] != nil {
				t.Fatal("duplicate registry tool")
			}
			out[name] = row
		}
		return out
	}
	before, after := read("assistant-tools-v1.json"), read("assistant-tools-v2.json")
	if len(before) != 19 || len(after) != 61 {
		t.Fatal("unexpected reviewed registry scope")
	}
	for name, row := range before {
		if !reflect.DeepEqual(after[name], row) {
			t.Fatalf("prior reviewed tool changed: %s", name)
		}
	}
	provider, err := NewSearchProvider(SearchConfig{Provider: "exa", Endpoint: "http://127.0.0.1:1", APIKey: "synthetic-exa-key", LocalHTTP: true})
	if err != nil {
		t.Fatal(err)
	}
	service := &Service{dependencies: Dependencies{Search: provider}}
	for _, binding := range service.AvailableBindings() {
		if binding.Name != "search_web" {
			continue
		}
		hash := sha256.Sum256(binding.Parameters)
		expected := map[string]any{
			"name": binding.Name, "description": binding.Description,
			"version": binding.Version, "domain": "extension",
			"schema_json": string(binding.Parameters), "schema_sha256": hex.EncodeToString(hash[:]),
			"price_version": binding.PriceVersion, "price_sha256": binding.PriceSHA256,
		}
		if !reflect.DeepEqual(after["search_web"], expected) {
			t.Fatal("installed search registry differs from actual available binding")
		}
		return
	}
	t.Fatal("configured search is unavailable")
}

// Control execution is local to Core. These descriptors must never be routed
// to the Go extension dispatcher or silently replace any of the first 21 rows.
func TestReviewedCoreControlRegistryHasBoundedExactSchemas(t *testing.T) {
	raw, err := os.ReadFile("../../../../../contracts/assistant-tools-v2.json")
	if err != nil {
		t.Fatal(err)
	}
	const priorPrefixBytes = 20861
	const priorPrefixSHA256 = "fed7a5484dfb482694d7f54e4c858699f183ededf4b8a3562d9da60f1838441a"
	if len(raw) < priorPrefixBytes {
		t.Fatal("prior reviewed registry truncated")
	}
	prefix := sha256.Sum256(raw[:priorPrefixBytes])
	if hex.EncodeToString(prefix[:]) != priorPrefixSHA256 {
		t.Fatal("prior 21 reviewed tools changed")
	}
	var artifact struct {
		Tools []map[string]any `json:"tools"`
	}
	if json.Unmarshal(raw, &artifact) != nil || len(artifact.Tools) != 61 {
		t.Fatal("unexpected registry scope")
	}
	var expected []map[string]any
	if json.Unmarshal([]byte(`[{"description":"Load argument definitions for 1-8 exact names from the available tool directory. Select only tools needed for the current task. Definitions appear in the next request; this does not execute the selected tools or authorize account actions. Already loaded tools need not be loaded again.","domain":"registered_core","name":"discover_tools","price_sha256":"5d6fb460c679922a1976e87ef729440d5394187e9b12331f33faf12904a64192","price_version":"builtin-free-v1","schema_json":"{\"additionalProperties\":false,\"properties\":{\"names\":{\"items\":{\"maxLength\":80,\"minLength\":1,\"type\":\"string\"},\"maxItems\":8,\"minItems\":1,\"type\":\"array\",\"uniqueItems\":true}},\"required\":[\"names\"],\"type\":\"object\"}","schema_sha256":"2cbcd85ab6df0cc70a46d3fde91ccdf4a76da801625839083c77116adab39877","version":"core-agent-control-v1"},{"description":"Deliver a concise final message and immediately stop this run, including remaining tool calls. Use when the task is complete, the user asks to stop, or no useful next step is possible. State incomplete work and unconfirmed actions. This does not block future messages, restrict the account, cancel completed actions, or contact support. A normal final text answer also ends the run without this tool.","domain":"registered_core","name":"end_conversation","price_sha256":"5d6fb460c679922a1976e87ef729440d5394187e9b12331f33faf12904a64192","price_version":"builtin-free-v1","schema_json":"{\"additionalProperties\":false,\"properties\":{\"message\":{\"maxLength\":2000,\"minLength\":1,\"type\":\"string\"},\"reason\":{\"enum\":[\"completed\",\"user_requested\",\"cannot_proceed\"],\"type\":\"string\"}},\"required\":[\"reason\",\"message\"],\"type\":\"object\"}","schema_sha256":"20a254743dd21530d8e5b3439bc3309ef2757fe67a36a6cc88531b74c7dd4609","version":"core-agent-control-v1"}]`), &expected) != nil {
		t.Fatal("invalid test contract")
	}
	if !reflect.DeepEqual(artifact.Tools[21:23], expected) {
		t.Fatal("Core control descriptor differs from reviewed executor contract")
	}
	for _, binding := range (&Service{}).AvailableBindings() {
		if binding.Name == "discover_tools" || binding.Name == "end_conversation" {
			t.Fatal("Core control exposed as extension execution")
		}
	}
	for _, row := range expected {
		name := row["name"].(string)
		schemaJSON := row["schema_json"].(string)
		digest := sha256.Sum256([]byte(schemaJSON))
		if hex.EncodeToString(digest[:]) != row["schema_sha256"] {
			t.Fatal("control schema digest mismatch")
		}
		var schema any
		if json.Unmarshal([]byte(schemaJSON), &schema) != nil {
			t.Fatal("invalid control schema")
		}
		compiler := jsonschema.NewCompiler()
		const location = "https://lmm.invalid/reviewed-agent-control"
		if err := compiler.AddResource(location, schema); err != nil {
			t.Fatal(err)
		}
		compiled, err := compiler.Compile(location)
		if err != nil {
			t.Fatal(err)
		}
		var cases []struct {
			label string
			args  any
			valid bool
		}
		if name == "discover_tools" {
			cases = []struct {
				label string
				args  any
				valid bool
			}{
				{"one exact name", map[string]any{"names": []any{"search_web"}}, true},
				{"eight names", map[string]any{"names": []any{"a", "b", "c", "d", "e", "f", "g", "h"}}, true},
				{"empty batch", map[string]any{"names": []any{}}, false},
				{"nine names", map[string]any{"names": []any{"a", "b", "c", "d", "e", "f", "g", "h", "i"}}, false},
				{"duplicate names", map[string]any{"names": []any{"search_web", "search_web"}}, false},
				{"non-string", map[string]any{"names": []any{1}}, false},
				{"undeclared authority", map[string]any{"names": []any{"search_web"}, "grant": true}, false},
			}
		} else {
			cases = []struct {
				label string
				args  any
				valid bool
			}{
				{"normal completion", map[string]any{"reason": "completed", "message": "Finished."}, true},
				{"user stop", map[string]any{"reason": "user_requested", "message": "Stopped."}, true},
				{"cannot continue", map[string]any{"reason": "cannot_proceed", "message": "The requested action remains incomplete."}, true},
				{"max UTF-8 text", map[string]any{"reason": "completed", "message": strings.Repeat("😀", 2000)}, true},
				{"unknown reason", map[string]any{"reason": "close_account", "message": "No."}, false},
				{"missing reason", map[string]any{"message": "Done."}, false},
				{"empty final text", map[string]any{"reason": "completed", "message": ""}, false},
				{"oversized text", map[string]any{"reason": "completed", "message": strings.Repeat("x", 2001)}, false},
				{"undeclared authority", map[string]any{"reason": "completed", "message": "Done.", "disable_user": true}, false},
			}
		}
		for _, c := range cases {
			t.Run(name+"/"+c.label, func(t *testing.T) {
				if (compiled.Validate(c.args) == nil) != c.valid {
					t.Fatal("control argument boundary changed")
				}
			})
		}
	}
}
