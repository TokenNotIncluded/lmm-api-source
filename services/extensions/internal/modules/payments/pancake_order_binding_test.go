package payments

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestPancakeCallbackUsesOriginalOrderBinding(t *testing.T) {
	c := Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD", Enabled: false}
	p := &Pancake{cfg: PancakeConfig{Channel: c, StoreID: "new_store", ProductID: "new_product"}}
	for _, tc := range []struct {
		name, payload string
		channel       int64
		want          error
	}{
		{"original_disabled_channel", pancakePaid, c.ID, ErrEvidence},
		{"replacement_product", strings.Replace(pancakePaid, "PROD_0123456789abcdefghijkl", "new_product", 1), c.ID, ErrConflict},
		{"replacement_store", strings.Replace(pancakePaid, "STO_0123456789abcdefghijkl", "new_store", 1), c.ID, ErrConflict},
		{"other_channel", pancakePaid, c.ID + 1, ErrConflict},
	} {
		t.Run(tc.name, func(t *testing.T) {
			memory := newMemory()
			o := gatewayOrder(c)
			o.ChannelID = tc.channel
			if err := memory.Insert(context.Background(), o); err != nil {
				t.Fatal(err)
			}
			core := newCore(c)
			s, err := New(memory, callbackRejectProofCore{Core: core}, p)
			if err != nil {
				t.Fatal(err)
			}
			err = s.Receive(context.Background(), c.ID, gatewayCallback(tc.payload, "X-Waffo-Signature"))
			if !errors.Is(err, tc.want) {
				t.Fatalf("got %v, want %v", err, tc.want)
			}
			if len(memory.events) != 0 || core.count("credit") != 0 {
				t.Fatal("callback bypassed Core proof admission")
			}
		})
	}
}

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
