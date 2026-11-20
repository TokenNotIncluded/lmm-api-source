// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"errors"
	"reflect"
	"testing"
)

// Exercise the view policy without substituting for persistence or Core RPC.
func TestMerchantProductViewRetainsConfiguredPaymentMethods(t *testing.T) {
	s := &Service{funds: DisabledFunds{}}
	for _, status := range []string{"draft", "paused", "published"} {
		t.Run(status, func(t *testing.T) {
			row := LocatedProduct{
				ShopID: "shop", Owner: Account{Kind: "personal", ID: 1},
				Product: Product{
					ID: "product", Title: "Configured balance", Published: status == "published",
					Legacy: &LegacyProductMetadata{
						Status: status, Approved: true, Visibility: "public",
						PaymentMethods: []string{"balance"}, DefaultVariantID: "variant",
					},
				},
				Variants: []Variant{{
					ID: "variant", ProductID: "product",
					Price: Price{Amount: 500000, Unit: "credits"}, Quota: -1,
					Legacy: &LegacyVariantMetadata{Enabled: true, Template: "fixed-content"},
				}},
			}
			merchant, err := s.merchantProductView(row, 1)
			must(t, err)
			if !reflect.DeepEqual(merchant["payment_methods"], []string{"balance"}) || merchant["trading_paused"] != true {
				t.Fatalf("merchant lost its configuration or enabled checkout: %#v", merchant)
			}
			saved, err := s.publicLegacyViewFor(row, false, 2)
			must(t, err)
			if !reflect.DeepEqual(saved["payment_methods"], []string{}) || saved["trading_paused"] != true {
				t.Fatalf("saved-product view enabled paused payments: %#v", saved)
			}
			public, err := s.publicLegacyViewFor(row, true, 2)
			if status != "published" {
				if !errors.Is(err, ErrNotFound) {
					t.Fatalf("unpublished product became public: %v", err)
				}
			} else {
				must(t, err)
				if !reflect.DeepEqual(public["payment_methods"], []string{}) || public["trading_paused"] != true {
					t.Fatalf("public view enabled unavailable payments: %#v", public)
				}
			}
		})
	}
}
