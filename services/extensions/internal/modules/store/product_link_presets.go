// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"strings"
)

const productLinkPresetsKey = "product-link-presets"

type productLinkPreset struct {
	ID string `json:"id"`
	LegacyLink
}

type productLinkPresets struct {
	Presets []productLinkPreset `json:"presets"`
}

func (s *Service) saveProductLinkPresets(ctx context.Context, token string, body json.RawMessage) error {
	auth, ok := s.auth.(administrativeAuthority)
	if !ok {
		return ErrUnavailable
	}
	if _, err := auth.CheckAdmin(ctx, token, true); err != nil {
		return err
	}
	var input productLinkPresets
	if legacyDecode(body, &input) != nil || input.Presets == nil || len(input.Presets) > 50 {
		return ErrInvalid
	}
	seen := make(map[string]bool, len(input.Presets))
	for _, p := range input.Presets {
		if !Key(p.ID) || seen[p.ID] || !Text(p.Title, 200) || len(p.Description) > 4000 ||
			strings.ContainsRune(p.Description, 0) || len(p.URL) > 4096 || !deliveryURL(p.URL) {
			return ErrInvalid
		}
		seen[p.ID] = true
	}
	return s.repo.Within(ctx, pricePolicyScope, func(tx Tx) error {
		return tx.Put("commands", productLinkPresetsKey, input)
	})
}
