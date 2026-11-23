package assistant_services

import (
	"encoding/json"
	"math"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

func TestCalculateCostUsesSuppliedUSDPricesOnce(t *testing.T) {
	for _, test := range []struct {
		ratio string
		want  float64
	}{{"", 1.5}, {`,"group_ratio":0.5`, 0.75}, {`,"group_ratio":0`, 0}, {`,"group_ratio":1`, 1.5}} {
		out, err := calculateCost([]byte(`{"input_tokens":1000000,"output_tokens":250000,"input_usd_per_million":1,"output_usd_per_million":2` + test.ratio + `}`))
		if err != nil || math.Abs(out["total_cost_usd"].(float64)-test.want) > 1e-12 || out["pricing_currency"] != "USD" || out["pricing_schema_version"] != 2 || out["estimate_only"] != true || out["price_source"] != "supplied_parameters" {
			t.Fatal("estimate contract changed")
		}
	}
}

func TestCalculateCostRejectsMissingNullAndNonfiniteInput(t *testing.T) {
	good := map[string]any{"input_tokens": 1, "output_tokens": 1, "input_usd_per_million": 1, "output_usd_per_million": 1}
	for key := range good {
		for _, value := range []any{nil, "1", -1, []any{1}, true} {
			in := map[string]any{}
			for k, v := range good {
				in[k] = v
			}
			in[key] = value
			raw, _ := json.Marshal(in)
			if _, err := calculateCost(raw); err != access.ErrInvalid {
				t.Fatalf("invalid required field accepted: %s", key)
			}
		}
		in := map[string]any{}
		for k, v := range good {
			if k != key {
				in[k] = v
			}
		}
		raw, _ := json.Marshal(in)
		if _, err := calculateCost(raw); err != access.ErrInvalid {
			t.Fatal("missing value defaulted to zero")
		}
	}
	for _, raw := range []string{`null`, `[]`, `{"input_tokens":1,"input_tokens":2}`, `{"input_tokens":1e309,"output_tokens":1,"input_usd_per_million":1,"output_usd_per_million":1}`, `{"input_tokens":1e308,"output_tokens":1e308,"input_usd_per_million":1e308,"output_usd_per_million":1e308}`, `{"input_tokens":1,"output_tokens":1,"input_usd_per_million":1,"output_usd_per_million":1,"group_ratio":null}`, `{"input_tokens":1,"output_tokens":1,"input_usd_per_million":1,"output_usd_per_million":1,"user_id":2}`} {
		if _, err := calculateCost([]byte(raw)); err != access.ErrInvalid {
			t.Fatal("unbounded estimate accepted")
		}
	}
}
