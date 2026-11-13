package store

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestPickupMailFrozenTransactionAndLegacyPickup(t *testing.T) {
	ctx := context.Background()
	f, policy := limitFixture(t, 100, 10)
	cfg := PickupSMTPConfig{Host: "smtp.example.test", Port: 465, TLSMode: "implicit", From: "store@example.test", Origin: "https://store.example.test", KeyFile: "/private/mail"}
	sender, err := NewPickupSMTP(cfg, "", nil)
	must(t, err)
	key := []byte(strings.Repeat("k", 32))
	must(t, f.s.WithPickupMail(sender, key, cfg.Origin))
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var p Product
		must(t, tx.Get("products", f.product.ID, &p))
		p.Legacy.EmailPickupLink = true
		return tx.Put("products", p.ID, p)
	}))
	promo := testPromotion(f, 10000, 20)
	writePromotion(t, f, promo)
	req := Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 1, Key: "mail-free", Policy: policy, PromotionCode: promo.Code, RequireFree: true, PickupEmail: "synthetic@example.test"}
	o, err := f.s.Purchase(ctx, buyerToken, req)
	must(t, err)
	var frozen pickupMailRecord
	var payload pickupMailPayload
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error { var e error; frozen, payload, e = f.s.openPickupMail(tx, f.shop.ID, o.ID); return e }))
	if payload.CreatedAt != o.Legacy.CreatedAt || payload.CreatedAt <= 0 {
		t.Fatal("mail date not frozen at order preparation")
	}
	again, err := f.s.Purchase(ctx, buyerToken, req)
	must(t, err)
	if again.ID != o.ID {
		t.Fatal("retry changed order")
	}
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var r pickupMailRecord
		must(t, tx.Get("commands", pickupMailID(o.ID), &r))
		if r != frozen {
			t.Fatal("retry changed frozen ciphertext")
		}
		return nil
	}))
	req.PickupEmail = "different@example.test"
	_, err = f.s.Purchase(ctx, buyerToken, req)
	wantError(t, err, ErrConflict)
	raw, _ := json.Marshal(frozen)
	if strings.Contains(string(raw), payload.Token) || strings.Contains(string(raw), payload.Recipient) {
		t.Fatal("plaintext in outbox")
	}
	indexed := &a06RefundIndex{memoryRepo: f.repo, shop: f.shop.ID}
	service, err := New(indexed, f.auth, nil)
	must(t, err)
	must(t, service.WithPickupMail(sender, key, cfg.Origin))
	v, err := service.Pickup(ctx, "", 0, payload.Token, "", true)
	must(t, err)
	if v["fixed_content"] != "Synthetic delivery" {
		t.Fatal("mail bearer claim failed")
	}
	// Existing plaintext pickup records remain readable and are not migrated.
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		return tx.Put("commands", ID("store-pickup-v1", o.ID), pickupLink{Token: payload.Token})
	}))
	_, err = service.Pickup(ctx, "", 0, payload.Token, "", true)
	must(t, err)
	must(t, f.repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var row Order
		must(t, tx.Get("orders", o.ID, &row))
		row.State = Refunded
		return tx.Put("orders", row.ID, row)
	}))
	_, err = service.Pickup(ctx, "", 0, payload.Token, "", true)
	if !errors.Is(err, ErrConflict) {
		t.Fatal("mail link bypassed refund")
	}
	if len(f.funds.commands) != 0 {
		t.Fatal("mail/free claim touched money")
	}
}
