package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"reflect"
	"testing"
	"time"
)

// The controlled Core boundary supplies a canonical quote. No Go pricing or
// signature/financial admission is simulated by this storage/adapter test.
type quantityQuoteCore struct {
	*fakeCore
	amount         int64
	creditOverride int64
}

func (c *quantityQuoteCore) Prepare(ctx context.Context, user string, r PrepareRequest) (Intent, error) {
	if _, exists := c.intents[r.Key]; exists {
		return c.fakeCore.Prepare(ctx, user, r)
	}
	v, err := c.fakeCore.Prepare(ctx, user, r)
	if err != nil {
		return v, err
	}
	v.AmountMinor = c.amount
	v.CreditUnits = r.RequestedCreditUnits
	if c.creditOverride != 0 {
		v.CreditUnits = c.creditOverride
	}
	raw, _ := json.Marshal(r)
	h := sha256.Sum256(raw)
	v.RequestHash = h[:]
	c.intents[r.Key] = v
	return v, nil
}
func (c *quantityQuoteCore) PreparePersonal(ctx context.Context, user string, r PrepareRequest) (Intent, error) {
	r.AccountID = 99
	return c.Prepare(ctx, user, r)
}

type quantityQuoteAdapter struct {
	*fakeAdapter
	sent []Intent
}

func (a *quantityQuoteAdapter) Checkout(ctx context.Context, o Order, key string) (Checkout, error) {
	a.sent = append(a.sent, o.Intent)
	return a.fakeAdapter.Checkout(ctx, o, key)
}
func quantityFixture(t *testing.T, store Store) (*Service, *quantityQuoteCore, *quantityQuoteAdapter) {
	t.Helper()
	channel := Channel{ID: 3, Provider: "waffo_pancake", Merchant: "synthetic_merchant", Environment: "test", Currency: "CNY", Enabled: true, CreditQuantity: true}
	core := &quantityQuoteCore{fakeCore: newCore(channel), amount: 1327}
	adapter := &quantityQuoteAdapter{fakeAdapter: &fakeAdapter{c: channel, window: time.Hour}}
	service, err := New(store, core, adapter)
	if err != nil {
		t.Fatal(err)
	}
	return service, core, adapter
}
func TestCreditQuantityCanonicalQuoteAndReplay(t *testing.T) {
	ctx := context.Background()
	store := newMemory()
	service, core, adapter := quantityFixture(t, store)
	req := PrepareRequest{Key: "quantity_request_0001", ChannelID: 3, Currency: "CNY", RequestedCreditUnits: 1000000}
	quote, err := service.Preview(ctx, "user", req)
	if err != nil || quote.AmountMinor != 1327 || quote.CreditUnits != 1000000 || len(adapter.sent) != 0 {
		t.Fatal(quote, err)
	}
	req.AccountID = quote.AccountID
	order, err := service.Create(ctx, "user", req)
	if err != nil {
		t.Fatal(err)
	}
	core.amount = 9999 // A later Core policy must not rewrite a frozen intent.
	duplicate, err := service.Create(ctx, "user", req)
	if err != nil || !reflect.DeepEqual(order.Intent, duplicate.Intent) {
		t.Fatal("frozen quote changed", duplicate, err)
	}
	restart, err := New(store, core, adapter)
	if err != nil {
		t.Fatal(err)
	}
	if err = restart.Work(ctx, 100); err != nil {
		t.Fatal(err)
	}
	if len(adapter.sent) != 1 || adapter.sent[0].AmountMinor != 1327 || adapter.sent[0].CreditUnits != 1000000 || adapter.sent[0].RequestedCreditUnits != 1000000 || !bytes.Equal(adapter.sent[0].RequestHash, order.Intent.RequestHash) {
		t.Fatal("provider did not receive exact frozen intent", adapter.sent)
	}
	req.RequestedCreditUnits++
	if _, err = restart.Create(ctx, "user", req); !errors.Is(err, ErrConflict) {
		t.Fatal("changed quantity reused identity", err)
	}
}
func TestCreditQuantityRejectsWrongCorePriceOrCredit(t *testing.T) {
	for _, tc := range []struct {
		name                                          string
		requested, charge, outputCredit, outputAmount int64
	}{
		{"asserted_price_mismatch", 1000000, 1326, 0, 1327},
		{"credit_mismatch", 1000000, 0, 999999, 1327},
		{"nonpositive_core_charge", 1000000, 0, 0, 0},
		{"negative_quantity", -1, 1327, 0, 1327},
		{"zero_amount_without_quantity", 0, 0, 0, 1327},
	} {
		t.Run(tc.name, func(t *testing.T) {
			service, core, adapter := quantityFixture(t, newMemory())
			core.amount = tc.outputAmount
			core.creditOverride = tc.outputCredit
			_, err := service.Create(context.Background(), "user", PrepareRequest{Key: "quantity_invalid_0001", AccountID: 99, ChannelID: 3, Currency: "CNY", AmountMinor: tc.charge, RequestedCreditUnits: tc.requested})
			if err == nil || len(adapter.sent) != 0 {
				t.Fatal("invalid quote accepted", err)
			}
		})
	}
}
