package store

import (
	"context"
	"testing"
)

func TestMinimumUnitPriceBoundariesAndRepeatedWrites(t *testing.T) {
	ctx := context.Background()
	svc := &Service{repo: newRepo()}
	current := func(want int64) {
		t.Helper()
		actual, _, err := svc.currentStorePrices(ctx)
		must(t, err)
		if actual != want {
			t.Fatalf("floor = %d; want %d", actual, want)
		}
	}
	current(defaultMinimumUnitPrice)
	for _, floor := range []int64{25, 25, 0, legacySafeInteger} {
		must(t, svc.repo.Within(ctx, pricePolicyScope, func(tx Tx) error { return tx.Put("commands", pricePolicyKey, pricePolicy{Minimum: &floor}) }))
		current(floor)
		must(t, svc.repo.Within(ctx, "synthetic-merchant", func(tx Tx) error {
			actual, err := tx.MinimumUnitPrice()
			if err != nil {
				return err
			}
			if actual != floor || validListingPrice(0, actual) || validListingPrice(floor-1, actual) || !validListingPrice(legacySafeInteger, actual) || validListingPrice(legacySafeInteger+1, actual) {
				t.Fatalf("price range inconsistent at floor %d", floor)
			}
			return nil
		}))
	}
}
