// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package rustfunds

import (
	"context"
	"errors"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func TestPartialRefundRequiresNegotiatedCapabilityBeforeDispatch(t *testing.T) {
	core := &coreStub{}
	adapter, err := New(core, features)
	if err != nil {
		t.Fatal(err)
	}
	c := command(store.Price{Amount: 100, Unit: "credits"})
	c.Kind = "refund"
	c.Refund = &store.RefundSpec{ID: "refund", Amount: 20, Mode: "amount"}
	if _, err = adapter.Execute(context.Background(), "seller", c); !errors.Is(err, store.ErrPaymentUnavailable) || core.execute != 0 {
		t.Fatal(err, core.execute)
	}
	adapter, err = New(core, append(append([]string{}, features...), "commerce.refund.partial.v1"))
	if err != nil {
		t.Fatal(err)
	}
	if !adapter.PartialRefundAvailable() {
		t.Fatal("negotiated partial refunds unavailable")
	}
	if _, err = adapter.Execute(context.Background(), "seller", c); err != nil || core.execute != 1 {
		t.Fatal(err, core.execute)
	}
}
