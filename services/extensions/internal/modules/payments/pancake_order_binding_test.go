package payments

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
)

func TestPancakeOrderBindingSurvivesConfigurationChange(t *testing.T) {
	c := Channel{ID: 1, Provider: "waffo_pancake", Merchant: "merchant", Environment: "test", Currency: "USD", Enabled: true}
	p := &Pancake{cfg: PancakeConfig{Channel: c, StoreID: "new_store", ProductID: "new_product"}}
	o := Order{ID: "top_1", ChannelID: 1, Namespace: c.namespace(), Intent: Intent{ID: 1, AmountMinor: 100, Provider: c.Provider, Merchant: c.Merchant, Environment: c.Environment, Currency: c.Currency}, PancakeBinding: &PancakeOrderBinding{StoreID: "old_store", ProductID: "old_product", TaxCategory: "saas"}}
	raw, err := json.Marshal(o)
	if err != nil {
		t.Fatal(err)
	}
	var restored Order
	if err = json.Unmarshal(raw, &restored); err != nil {
		t.Fatal(err)
	}
	bound, err := p.forOrder(restored)
	if err != nil {
		t.Fatal(err)
	}
	if bound.cfg.StoreID != "old_store" || bound.cfg.ProductID != "old_product" || p.cfg.ProductID != "new_product" {
		t.Fatal("original binding lost or shared config mutated")
	}
	changed := restored
	changed.PancakeBinding = &PancakeOrderBinding{StoreID: "new_store", ProductID: "new_product"}
	if sameIdentity(restored, changed) {
		t.Fatal("stored binding was mutable")
	}
	restored.Intent.Merchant = "other"
	if _, err = p.forOrder(restored); !errors.Is(err, ErrConflict) {
		t.Fatal("cross merchant accepted", err)
	}
}

func TestPancakeHistoricalOrderWithoutBindingStopsBeforeIO(t *testing.T) {
	p := &Pancake{}
	o := Order{ID: "historical"}
	if _, err := p.Checkout(context.Background(), o, "key"); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
	if _, err := p.Lookup(context.Background(), o); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
	raw, err := json.Marshal(o)
	if err != nil {
		t.Fatal(err)
	}
	var restored Order
	if err = json.Unmarshal(raw, &restored); err != nil || restored.PancakeBinding != nil {
		t.Fatal("historical snapshot changed", err)
	}
}
