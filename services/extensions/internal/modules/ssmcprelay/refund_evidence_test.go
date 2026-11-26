package ssmcprelay

import (
	"context"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"testing"
	"time"
)

type refundUnknownFunds struct {
	*testFunds
	unknown string
	writes  int
}

func (f *refundUnknownFunds) LookupCommerceReceipt(ctx context.Context, t string, c store.MoneyCommand) (store.Receipt, error) {
	if f.unknown == "revoked" {
		return store.Receipt{}, store.ErrForbidden
	}
	if c.Kind == f.unknown {
		return store.Receipt{}, store.ErrPending
	}
	return f.testFunds.LookupCommerceReceipt(ctx, t, c)
}
func (f *refundUnknownFunds) Execute(ctx context.Context, t string, c store.MoneyCommand) (store.Receipt, error) {
	if c.Kind == "refund" {
		f.writes++
	}
	return f.testFunds.Execute(ctx, t, c)
}

type refundProbe struct {
	*testRelay
	err   error
	calls int
}

func (r *refundProbe) Grant(ctx context.Context, e Entitlement) error {
	r.calls++
	if r.err != nil {
		return r.err
	}
	return r.testRelay.Grant(ctx, e)
}
func TestExpiredRecoveryRequiresMatchedOriginalEvidence(t *testing.T) {
	for _, kind := range []string{"same period", "no payment", "payment query unknown", "wrong payment", "rejected payment", "grant unknown", "grant conflict", "already delivered", "refund query unknown", "settlement query unknown", "already captured", "revoked during lookup", "historical USD"} {
		t.Run(kind, func(t *testing.T) {
			ctx := context.Background()
			s, relay, base := fixture(t)
			f := &refundUnknownFunds{testFunds: base}
			s.funds = f
			g := &refundProbe{testRelay: relay}
			s.relay = g
			o, e := s.Preview(ctx, 2, "recovery", 1)
			if e != nil {
				t.Fatal(e)
			}
			if kind == "historical USD" {
				o.PricingVersion = 0
				o.TotalPriceQuota = 0
				o.UnitPriceQuotaPerGiB = 0
			}
			o.State = "payment_pending"
			if e = s.repo.Within(ctx, namespace(2), func(tx store.Tx) error { return tx.Put("commands", o.ID, o) }); e != nil {
				t.Fatal(e)
			}
			pay := o.command("pay")
			r := base.record(pay)
			s.now = func() time.Time { return time.Unix(o.ResetAt, 0) }
			relay.expired = true
			switch kind {
			case "same period":
				relay.expired = false
			case "no payment":
				delete(base.ops, pay.Key)
			case "payment query unknown":
				f.unknown = "pay"
			case "wrong payment":
				r.CommandHash = store.ID("another")
				base.ops[pay.Key] = r
			case "rejected payment":
				r.Outcome = "rejected"
				base.ops[pay.Key] = r
			case "grant unknown":
				g.err = ErrRelay
			case "grant conflict":
				g.err = ErrGrantConflict
			case "already delivered":
				relay.grants[o.ID] = o.entitlement()
			case "refund query unknown":
				f.unknown = "refund"
			case "settlement query unknown":
				f.unknown = "settle"
			case "already captured":
				x := o
				x.PaymentRef = r.Reference
				base.record(x.command("settle"))
			case "revoked during lookup":
				f.unknown = "revoked"
			}
			seen, e := s.RefundExpired(ctx, 2, "merchant", o.ID)
			if kind == "historical USD" {
				if e != nil || seen.State != "refunded" || f.writes != 1 {
					t.Fatal(seen, e)
				}
				cmd := seen.command("refund")
				if cmd.Total != (store.Price{Amount: 100, Currency: "USD"}) || base.ops[cmd.Key].CommandHash != store.ID(cmd) {
					t.Fatal("historical economics changed")
				}
				return
			}
			if e == nil || f.writes != 0 {
				t.Fatal(kind, seen, e, f.writes)
			}
			if kind == "revoked during lookup" && (!errors.Is(e, store.ErrForbidden) || seen != (Order{})) {
				t.Fatal("revoked caller received order", seen, e)
			}
			if kind == "already delivered" && seen.State != "settlement_pending" {
				t.Fatal(seen, e)
			}
			if (kind == "same period" || kind == "no payment" || kind == "payment query unknown" || kind == "wrong payment" || kind == "rejected payment") && g.calls != 0 {
				t.Fatal("grant probed without eligibility")
			}
		})
	}
}
