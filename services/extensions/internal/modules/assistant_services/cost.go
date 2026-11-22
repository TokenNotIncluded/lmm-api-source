package assistant_services

import (
	"encoding/json"
	"math"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

func costDefinition() PureDefinition {
	properties := map[string]any{}
	for _, name := range []string{"input_tokens", "output_tokens", "input_usd_per_million", "output_usd_per_million", "group_ratio"} {
		properties[name] = map[string]any{"type": "number", "minimum": 0}
	}
	return PureDefinition{Name: "calculate_cost", Description: "Estimate USD cost from supplied nonnegative token counts and per-million-token prices. Obtain exact live model/group prices first; already adjusted prices require group_ratio=1 so discounts are not applied twice. This deterministic estimate does not verify the supplied prices, reserve funds, quote checkout or post a bill.", Parameters: objectSchema(properties, []string{"input_tokens", "output_tokens", "input_usd_per_million", "output_usd_per_million"})}
}

func calculateCost(raw json.RawMessage) (map[string]any, error) {
	var input map[string]json.RawMessage
	if decode(raw, &input) != nil {
		return nil, access.ErrInvalid
	}
	allowed := map[string]bool{"input_tokens": true, "output_tokens": true, "input_usd_per_million": true, "output_usd_per_million": true, "group_ratio": true}
	values := map[string]float64{"group_ratio": 1}
	for name, value := range input {
		if !allowed[name] {
			return nil, access.ErrInvalid
		}
		var n float64
		if string(value) == "null" || json.Unmarshal(value, &n) != nil || n < 0 || math.IsNaN(n) || math.IsInf(n, 0) {
			return nil, access.ErrInvalid
		}
		values[name] = n
	}
	for _, name := range []string{"input_tokens", "output_tokens", "input_usd_per_million", "output_usd_per_million"} {
		if _, ok := input[name]; !ok {
			return nil, access.ErrInvalid
		}
	}
	ratio := values["group_ratio"]
	inputCost := values["input_tokens"] / 1_000_000 * values["input_usd_per_million"] * ratio
	outputCost := values["output_tokens"] / 1_000_000 * values["output_usd_per_million"] * ratio
	total := inputCost + outputCost
	if math.IsNaN(total) || math.IsInf(total, 0) {
		return nil, access.ErrInvalid
	}
	return map[string]any{"ok": true, "input_cost_usd": inputCost, "output_cost_usd": outputCost, "total_cost_usd": total, "pricing_currency": "USD", "pricing_schema_version": 2, "group_ratio": ratio, "formula": "(input_tokens / 1,000,000 × input price + output_tokens / 1,000,000 × output price) × group ratio", "estimate_only": true, "price_source": "supplied_parameters"}, nil
}
