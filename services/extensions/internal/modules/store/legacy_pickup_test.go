// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"testing"

	"golang.org/x/crypto/bcrypt"
)

type launchPickupIndex struct {
	*a06RefundIndex
	oldHash, canonical string
}

func (r *launchPickupIndex) LegacyPickupToken(_ context.Context, hash string) (string, error) {
	if hash != r.oldHash {
		return "", ErrNotFound
	}
	return r.canonical, nil
}

func TestLegacyPickupBcryptCannotBeDowngradedOrMixedWithNativeProtection(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("old-pickup-code"), 4)
	must(t, err)
	p := &pickupProtection{LegacyBcrypt: string(hash)}
	if !validPickupCode(p, "old-pickup-code") || validPickupCode(p, "wrong-pickup-code") || validPickupCode(p, "") {
		t.Fatal("old code semantics changed")
	}
	p.Salt = "native-salt"
	if validPickupCode(p, "old-pickup-code") {
		t.Fatal("mixed verifier was accepted")
	}
	p.Salt = ""
	p.LegacyBcrypt = "$2a$31$malformed"
	if validPickupCode(p, "old-pickup-code") {
		t.Fatal("invalid/unbounded legacy cost accepted")
	}
}

func TestLegacyPickupAliasStillRequiresOriginalBuyerCodeAndUnrefundedState(t *testing.T) {
	f := a06StateSetup(t)
	old := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	oldHash := sha256.Sum256([]byte(old))
	secret := sha256.Sum256([]byte("store-import-pickup-v1\x00" + old))
	canonical := f.id + "." + hex.EncodeToString(secret[:])
	index := &launchPickupIndex{a06RefundIndex: &a06RefundIndex{memoryRepo: f.repo, shop: f.shop}, oldHash: hex.EncodeToString(oldHash[:]), canonical: canonical}
	var err error
	f.s, err = New(index, f.auth, f.funds)
	must(t, err)
	must(t, f.s.WithStockKey(make([]byte, 32)))
	sealed, err := f.s.stock.seal(f.shop, "old-unit", "PRIVATE-OLD-ITEM")
	must(t, err)
	hash, err := bcrypt.GenerateFromPassword([]byte("old-pickup-code"), 4)
	must(t, err)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.State = Settled
		o.Delivered = true
		o.StockItems = []string{"old-unit"}
		o.Delivery = stockDeliverySummary(o.StockItems)
		o.Legacy.DeliveryTemplate = "card-key"
		o.Legacy.PickupCodeRequired = true
		o.Imported = &ImportMarker{Source: "go-v0.2.118", Snapshot: "original-source-digest"}
		if err := tx.Put("orders", o.ID, o); err != nil {
			return err
		}
		if err := tx.Put("commands", ID("store-pickup-v1", o.ID), pickupLink{Token: canonical}); err != nil {
			return err
		}
		if err := tx.Put("commands", ID("legacy-order-pickup-v1", o.ID), pickupProtection{LegacyBcrypt: string(hash)}); err != nil {
			return err
		}
		return tx.Put("stock_items", "old-unit", StockItem{ID: "old-unit", OrderID: o.ID, VariantID: o.VariantID, State: stockDelivered, Ciphertext: sealed})
	}))
	_, err = f.s.Pickup(context.Background(), "", 0, old, "old-pickup-code", true)
	wantError(t, err, ErrUnauthorized)
	_, err = f.s.Pickup(context.Background(), otherToken, 3, old, "old-pickup-code", true)
	wantError(t, err, ErrNotFound)
	_, err = f.s.Pickup(context.Background(), buyerToken, 2, old, "wrong-pickup-code", true)
	wantError(t, err, ErrForbidden)
	v, err := f.s.Pickup(context.Background(), buyerToken, 2, old, "old-pickup-code", true)
	must(t, err)
	items := v["items"].([]string)
	if len(items) != 1 || items[0] != "PRIVATE-OLD-ITEM" || v["fixed_content"] != "" {
		t.Fatal("old encrypted delivery was not recovered")
	}
	_, err = f.s.Pickup(context.Background(), buyerToken, 2, canonical, "old-pickup-code", true)
	must(t, err)
	must(t, f.repo.Within(context.Background(), f.shop, func(tx Tx) error {
		var o Order
		if err := tx.Get("orders", f.id, &o); err != nil {
			return err
		}
		o.State = Refunded
		return tx.Put("orders", f.id, o)
	}))
	_, err = f.s.Pickup(context.Background(), buyerToken, 2, old, "old-pickup-code", true)
	wantError(t, err, ErrConflict)
	if f.funds.execCalls.Load() != 0 || f.funds.captureCalls.Load() != 0 {
		t.Fatal("historical pickup fabricated a pay/capture")
	}
}
