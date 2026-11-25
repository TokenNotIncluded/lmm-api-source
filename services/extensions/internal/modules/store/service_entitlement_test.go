package store

import (
	"context"
	"errors"
	"testing"
)

func TestServiceEntitlementFreezesPurchaseAndSerializesRefund(t *testing.T) {
	ctx := context.Background()
	f := setup(t, 20, 10, 10, false, true)
	core := &a06StateFunds{}
	f.s.funds = core
	price := Price{Amount: 23, Unit: "credits"}
	f.catalog(t, CatalogChange{Action: "create_variant", ProductID: f.product.ID, Title: "Traffic", Price: price, Quota: 10, Key: "traffic-sku"}, &f.variant)
	buyer := Account{Kind: "personal", ID: 2}
	p := Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: buyer, Quantity: 2, Key: "bound"}
	terms := ServiceTerms{Service: "ssmcpd.monthly-traffic.v1", Audience: ID("issuer"), Period: "2026-11", UnitsPerItem: 1000, UnitPrice: price}
	wrong := terms
	wrong.UnitPrice.Amount++
	if _, err := f.s.PurchaseServiceEntitlement(ctx, buyerToken, p, wrong); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	o, err := f.s.PurchaseServiceEntitlement(ctx, buyerToken, p, terms)
	must(t, err)
	wrong = terms
	wrong.Period = "2026-12"
	if _, err = f.s.PurchaseServiceEntitlement(ctx, buyerToken, p, wrong); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	if _, err = f.s.ClaimServiceDispatch(ctx, buyerToken, buyer, f.shop.ID, o.ID, terms.Service); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	_, err = f.change(buyerToken, o, "pay", "")
	must(t, err)
	if _, err = f.s.ClaimServiceDispatch(ctx, buyerToken, buyer, f.shop.ID, o.ID, terms.Service); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
	o, err = f.s.FinalizeDelivery(ctx, f.shop.ID, o.ID)
	must(t, err)
	if o.State != Settled || core.captureCalls.Load() != 1 {
		t.Fatal(o.State)
	}
	if _, err = f.s.ServiceOrder(ctx, sellerToken, Account{Kind: "personal", ID: 1}, f.shop.ID, o.ID, terms.Service); !errors.Is(err, ErrNotFound) {
		t.Fatal(err)
	}
	claim, err := f.s.ClaimServiceDispatch(ctx, buyerToken, buyer, f.shop.ID, o.ID, terms.Service)
	must(t, err)
	if claim.Entitlement.Dispatch != "unknown" {
		t.Fatal(claim)
	}
	_, err = f.change(buyerToken, o, "request_refund", "refund the original order")
	must(t, err)
	if _, err = f.change(sellerToken, o, "refund", ""); !errors.Is(err, ErrPending) {
		t.Fatal(err)
	}
	again, err := f.s.ClaimServiceDispatch(ctx, buyerToken, buyer, f.shop.ID, o.ID, terms.Service)
	must(t, err)
	if servicePassReference(f.shop.ID, claim) != servicePassReference(f.shop.ID, again) {
		t.Fatal("changed retry")
	}
	must(t, f.s.RecordServiceDispatch(ctx, f.shop.ID, claim, true))
	must(t, f.s.RecordServiceDispatch(ctx, f.shop.ID, again, true))
	o, err = f.change(sellerToken, o, "refund", "")
	must(t, err)
	if o.State != Refunded || o.Entitlement.Dispatch != "granted" {
		t.Fatal(o.State)
	}
	if _, err = f.s.ClaimServiceDispatch(ctx, buyerToken, buyer, f.shop.ID, o.ID, terms.Service); !errors.Is(err, ErrConflict) {
		t.Fatal(err)
	}
}
