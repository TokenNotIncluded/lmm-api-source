package payments

import (
	"context"
	"errors"
	"reflect"
	"testing"
)

func TestCreditQuantityPostgresFrozenRecovery(t *testing.T) {
	db := openPancakeRecoveryPG(t)
	if _, err := db.Exec(`DROP TABLE IF EXISTS payment_events,payment_orders,payment_storage_guard;` + Schema); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	store, err := NewPostgres(ctx, db, "lmm_payments_test")
	if err != nil {
		t.Fatal(err)
	}
	service, core, adapter := quantityFixture(t, store)
	req := PrepareRequest{Key: "quantity_pg_request_01", AccountID: 99, ChannelID: 3, Currency: "CNY", RequestedCreditUnits: 1000000}
	original, err := service.Create(ctx, "user", req)
	if err != nil {
		t.Fatal(err)
	}
	// Reopen the real PostgreSQL store. Worker recovery consumes its durable intent.
	reopened, err := NewPostgres(ctx, db, "lmm_payments_test")
	if err != nil {
		t.Fatal(err)
	}
	core.amount = 9999
	restart, err := New(reopened, core, adapter)
	if err != nil {
		t.Fatal(err)
	}
	if err = restart.Work(ctx, 100); err != nil {
		t.Fatal(err)
	}
	saved, err := reopened.Get(ctx, original.ID)
	if err != nil || !reflect.DeepEqual(saved.Intent, original.Intent) || len(adapter.sent) != 1 || adapter.sent[0].AmountMinor != 1327 || adapter.sent[0].CreditUnits != 1000000 {
		t.Fatal("durable canonical quote lost", saved, err)
	}
	duplicate, err := restart.Create(ctx, "user", req)
	if err != nil || !reflect.DeepEqual(duplicate.Intent, original.Intent) {
		t.Fatal("duplicate requoted", err)
	}
	if err = reopened.Mutate(ctx, original.ID, func(o *Order) error { o.Intent.RequestedCreditUnits++; return nil }); !errors.Is(err, ErrConflict) {
		t.Fatal("quantity provenance mutated", err)
	}
	bad := req
	bad.Key = "quantity_pg_wrongprice"
	bad.AmountMinor = 1326
	if _, err = restart.Create(ctx, "user", bad); !errors.Is(err, ErrConflict) {
		t.Fatal("wrong price accepted", err)
	}
	var count int
	if err = db.QueryRow(`SELECT count(*) FROM payment_orders`).Scan(&count); err != nil || count != 1 {
		t.Fatal("invalid quote persisted", count, err)
	}
}
