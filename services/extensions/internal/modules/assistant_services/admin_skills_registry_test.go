package assistant_services

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"reflect"
	"testing"

	"github.com/santhosh-tekuri/jsonschema/v6"
)

func TestAdminSkillInstalledRegistryMatchesActualBinding(t *testing.T) {
	raw, err := os.ReadFile("../../../../../contracts/assistant-tools-v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var file struct {
		Tools []map[string]string `json:"tools"`
	}
	if json.Unmarshal(raw, &file) != nil {
		t.Fatal("invalid registry")
	}
	count := 0
	for _, binding := range (&Service{}).Bindings() {
		if binding.Name != adminSkillsTool {
			continue
		}
		sum := sha256.Sum256(binding.Parameters)
		expected := map[string]string{"name": binding.Name, "description": binding.Description, "version": binding.Version, "domain": "extension", "schema_json": string(binding.Parameters), "schema_sha256": hex.EncodeToString(sum[:]), "price_version": binding.PriceVersion, "price_sha256": binding.PriceSHA256}
		found := 0
		for _, entry := range file.Tools {
			if entry["name"] == adminSkillsTool {
				found++
				if !reflect.DeepEqual(entry, expected) {
					t.Fatal("registry differs from actual executable binding")
				}
			}
		}
		if found != 1 {
			t.Fatal("missing or duplicate entry")
		}
		var schema any
		if json.Unmarshal(binding.Parameters, &schema) != nil {
			t.Fatal("invalid bound schema")
		}
		compiler := jsonschema.NewCompiler()
		if err := compiler.AddResource("https://lmm.invalid/admin-skills", schema); err != nil {
			t.Fatal(err)
		}
		compiled, err := compiler.Compile("https://lmm.invalid/admin-skills")
		if err != nil {
			t.Fatal(err)
		}
		for _, test := range []struct {
			input string
			valid bool
		}{
			{`{"target_user_id":202}`, true},
			{`{"target_user_id":202,"limit":1,"before_id":23}`, true},
			{`{"user_id":202}`, false},
			{`{"target_user_id":202,"limit":0}`, false},
			{`{"target_user_id":202,"limit":17}`, false},
			{`{"target_user_id":2147483648}`, false},
			{`{"target_user_id":202,"admin":true}`, false},
		} {
			var input any
			if json.Unmarshal([]byte(test.input), &input) != nil {
				t.Fatal("bad fixture")
			}
			if (compiled.Validate(input) == nil) != test.valid {
				t.Fatal("wrong schema admission", test.input)
			}
		}
		count++
	}
	if count != 1 {
		t.Fatal("actual admin skill binding missing")
	}
}
