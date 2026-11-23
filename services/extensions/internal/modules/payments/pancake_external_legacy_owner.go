// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"slices"
	"strings"

	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

// ExternalLegacyPancakeOwner declares that the old Go order namespaces in one
// store/environment are still settled by another deployment. It grants no Core
// settlement authority. Optional NativeProductIDs protects exclusive native SKUs;
// an empty list permits shared products under the explicit namespace ownership.
type ExternalLegacyPancakeOwner struct {
	StoreID          string   `toml:"store_id"`
	Environment      string   `toml:"environment"`
	NativeProductIDs []string `toml:"native_product_ids"`
}

func ValidateExternalLegacyPancakeOwners(owners []ExternalLegacyPancakeOwner) error {
	seen := make(map[string]bool, len(owners))
	for _, owner := range owners {
		key := owner.Environment + "/" + owner.StoreID
		if !validID(owner.StoreID) || (owner.Environment != "test" && owner.Environment != "live") ||
			seen[key] {
			return ErrInvalid
		}
		seen[key] = true
		products := make(map[string]bool, len(owner.NativeProductIDs))
		for _, product := range owner.NativeProductIDs {
			if !validID(product) || products[product] {
				return ErrInvalid
			}
			products[product] = true
		}
	}
	return nil
}

func externalLegacyPancakeReference(reference string) bool {
	if !validID(reference) {
		return false
	}
	for _, prefix := range []string{"WAFFO_PANCAKE-", "WAFFO_PANCAKE_SUB-"} {
		if strings.HasPrefix(reference, prefix) && len(reference) > len(prefix) {
			return true
		}
	}
	return false
}

func (owner ExternalLegacyPancakeOwner) matches(store, mode, eventType, reference string) bool {
	return owner.StoreID == store && eventType == "order.completed" && externalLegacyPancakeReference(reference) &&
		(owner.Environment == "test" && mode == "test" || owner.Environment == "live" && mode == "prod")
}

func (owner ExternalLegacyPancakeOwner) verify(raw, signature []byte, keys *pancake.WebhookPublicKeys) error {
	environment := pancake.EnvironmentTest
	if owner.Environment == "live" {
		environment = pancake.EnvironmentProd
	}
	// Never auto-detect the signing environment or relax the SDK replay window.
	// The public handler always supplies nil keys, selecting the official keys.
	event, err := pancake.VerifyWebhookTyped[pancake.WebhookEventData](string(raw), string(signature),
		&pancake.VerifyWebhookOptions{Environment: environment, PublicKeys: keys})
	if err != nil {
		return ErrInvalid
	}
	if event.Mode != environment || event.StoreID != owner.StoreID || event.EventType != "order.completed" ||
		event.Data.OrderMerchantExternalID == nil || !externalLegacyPancakeReference(*event.Data.OrderMerchantExternalID) {
		return ErrConflict
	}
	if !validID(event.ID) || !validID(event.Data.OrderID) {
		return ErrInvalid
	}
	if slices.Contains(owner.NativeProductIDs, event.Data.OrderMetadata["lmm_product_id"]) {
		return ErrConflict
	}
	return nil
}
