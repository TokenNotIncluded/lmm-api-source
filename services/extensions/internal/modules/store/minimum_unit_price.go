package store

import (
	"context"
	"encoding/json"
	"errors"
)

const defaultMinimumUnitPrice int64 = 500000
const pricePolicyScope = "@store-price-policy"
const pricePolicyKey = "minimum-unit-price"
const relayPriceKey = "relay-price-per-gib"
const DefaultRelayPricePerGiB int64 = 500000

type pricePolicy struct {
	Minimum *int64 `json:"minimum_unit_price_quota"`
}

func validListingPrice(price, floor int64) bool {
	return price > 0 && price >= floor && price <= legacySafeInteger
}

func (row LocatedProduct) listingFloor() int64 {
	if row.minimumUnitPrice == nil {
		return defaultMinimumUnitPrice
	}
	return *row.minimumUnitPrice
}

func readMinimumUnitPrice(tx Tx) (int64, error) {
	var policy pricePolicy
	err := tx.Get("commands", pricePolicyKey, &policy)
	if errors.Is(err, ErrNotFound) {
		return defaultMinimumUnitPrice, nil
	}
	if err != nil {
		return 0, err
	}
	if policy.Minimum == nil || *policy.Minimum < 0 || *policy.Minimum > legacySafeInteger {
		return 0, ErrUnavailable
	}
	return *policy.Minimum, nil
}

// A missing row means the pre-configuration tariff; zero is never a valid tariff.
type relayPricePolicy struct {
	Price int64 `json:"relay_price_quota_per_gib"`
}

func readRelayPricePerGiB(tx Tx) (int64, error) {
	var policy relayPricePolicy
	err := tx.Get("commands", relayPriceKey, &policy)
	price := policy.Price
	if errors.Is(err, ErrNotFound) {
		return DefaultRelayPricePerGiB, nil
	}
	if err != nil {
		return 0, err
	}
	if price <= 0 || price > legacySafeInteger {
		return 0, ErrUnavailable
	}
	return price, nil
}

// Configuration writes take the exclusive version through Repository.Within.
// The shared lock covers the first write as well as updates to an existing row.
func (t *postgresTx) lockedStorePrice(read func(Tx) (int64, error)) (int64, error) {
	if _, err := t.tx.ExecContext(t.ctx, "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))", "lmm_store:"+pricePolicyScope); err != nil {
		return 0, err
	}
	global := *t
	global.shop = pricePolicyScope
	return read(&global)
}

func (t *postgresTx) MinimumUnitPrice() (int64, error) {
	return t.lockedStorePrice(readMinimumUnitPrice)
}
func (t *postgresTx) RelayPricePerGiB() (int64, error) {
	return t.lockedStorePrice(readRelayPricePerGiB)
}

func (s *Service) setMinimumUnitPrice(ctx context.Context, token string, body json.RawMessage) error {
	auth, ok := s.auth.(administrativeAuthority)
	if !ok {
		return ErrUnavailable
	}
	if _, err := auth.CheckAdmin(ctx, token, true); err != nil {
		return err
	}
	var patch struct {
		Minimum   *int64          `json:"minimum_unit_price_quota"`
		Relay     json.RawMessage `json:"relay_price_quota_per_gib"`
		Fee       *int64          `json:"fee_bps"`
		Promotion *int64          `json:"promotion_quota"`
	}
	if err := legacyDecode(body, &patch); err != nil {
		return err
	}
	// Older consoles submit all three fields. The native host exposes a fixed
	// zero fee and promotion price, so reject requests that try to change either.
	if (patch.Fee != nil && *patch.Fee != 0) || (patch.Promotion != nil && *patch.Promotion != 0) {
		return ErrUnavailable
	}
	if patch.Minimum != nil && (*patch.Minimum < 0 || *patch.Minimum > legacySafeInteger) {
		return ErrInvalid
	}
	var relayPrice int64
	if patch.Relay != nil && (json.Unmarshal(patch.Relay, &relayPrice) != nil || relayPrice <= 0 || relayPrice > legacySafeInteger) {
		return ErrInvalid
	}
	if patch.Minimum == nil && patch.Relay == nil {
		return nil
	}
	return s.repo.Within(ctx, pricePolicyScope, func(tx Tx) error {
		if patch.Minimum != nil {
			if err := tx.Put("commands", pricePolicyKey, pricePolicy{Minimum: patch.Minimum}); err != nil {
				return err
			}
		}
		if patch.Relay != nil {
			return tx.Put("commands", relayPriceKey, relayPricePolicy{Price: relayPrice})
		}
		return nil
	})
}

func checkSubmittedPrices(tx Tx, product Product) error {
	floor, err := tx.MinimumUnitPrice()
	if err != nil {
		return err
	}
	variants, err := variantsFor(tx, product.ID)
	if err != nil {
		return err
	}
	for _, variant := range variants {
		if variant.Legacy != nil && (variant.Legacy.Enabled || variant.ID == product.Legacy.DefaultVariantID) && !validListingPrice(variant.Price.Amount, floor) {
			return ErrInvalid
		}
	}
	return nil
}
