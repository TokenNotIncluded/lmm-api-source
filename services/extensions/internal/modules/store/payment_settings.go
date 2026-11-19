// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

// paymentSettings describes the commerce adapter mounted by this process.
// Payment choices remain per product; this runtime has no seller gateway
// configuration or balance/fee read port. Never fabricate money values or accept
// credentials for an external payment path that it cannot execute.
func (s *Service) paymentSettings() map[string]any {
	available := s.funds.Available()
	method := map[string]any{
		"provider": "balance", "enabled": available, "configured": available,
		"category": "platform", "category_enabled": available,
		"effective_enabled": available, "has_key": false, "has_private_key": false,
	}
	if !available {
		method["unavailable_code"] = "payments_unavailable"
	}
	return map[string]any{
		"read_only": true, "external_eligible": false,
		"categories": map[string]bool{"platform_enabled": available, "external_enabled": false},
		"items":      []map[string]any{method},
	}
}
