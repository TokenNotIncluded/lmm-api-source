package pgtest

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"os"
	"strings"
	"testing"
	"time"

	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	_ "github.com/lib/pq"
)

// Real isolated PG18 migration/storage test. Core signatures/grants are covered
// by Rust tests, not replaced here with a synthetic paid/grant implementation.
func TestSubscriptionEPayOfflineUpgradeAndInbox(t *testing.T) {
	dsn := os.Getenv("SUBSCRIPTION_PAYMENT_TEST_DSN")
	if dsn == "" {
		t.Skip("dedicated empty *_test database required")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	var name string
	var empty bool
	if err = db.QueryRowContext(ctx, `SELECT current_database(),NOT EXISTS(SELECT FROM pg_class WHERE relnamespace='public'::regnamespace)`).Scan(&name, &empty); err != nil || !strings.HasSuffix(name, "_test") || !empty {
		t.Fatalf("private empty database required: %v %s %t", err, name, empty)
	}
	role := fmt.Sprintf("subscription_runtime_%d", os.Getpid())
	if _, err = db.ExecContext(ctx, `CREATE ROLE `+role+` NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;REVOKE CREATE ON SCHEMA public FROM PUBLIC;`+p.Schema+`GRANT USAGE ON SCHEMA public TO `+role+`;GRANT SELECT ON payment_storage_guard TO `+role+`;GRANT SELECT,INSERT,UPDATE ON payment_orders,payment_events TO `+role); err != nil {
		t.Fatal(err)
	}
	if _, err = db.ExecContext(ctx, `INSERT INTO payment_orders(id,namespace,version,snapshot) VALUES('original-order','epay/1001/test',1,'{"id":"original-order","namespace":"epay/1001/test","original":"preserve"}')`); err != nil {
		t.Fatal(err)
	}
	if _, err = p.UpgradeSubscriptionEPay(ctx, db, name+"_wrong", role); err == nil {
		t.Fatal("wrong database accepted")
	}
	if _, err = db.ExecContext(ctx, `ALTER ROLE `+role+` LOGIN`); err != nil {
		t.Fatal(err)
	}
	if _, err = p.UpgradeSubscriptionEPay(ctx, db, name, role); err == nil {
		t.Fatal("LOGIN runtime bypassed offline fence")
	}
	if _, err = db.ExecContext(ctx, `ALTER ROLE `+role+` NOLOGIN;CREATE ROLE `+role+`_delegate LOGIN;GRANT `+role+` TO `+role+`_delegate`); err != nil {
		t.Fatal(err)
	}
	if _, err = p.UpgradeSubscriptionEPay(ctx, db, name, role); err == nil {
		t.Fatal("external SET ROLE path bypassed offline fence")
	}
	if _, err = db.ExecContext(ctx, `REVOKE `+role+` FROM `+role+`_delegate;DROP ROLE `+role+`_delegate`); err != nil {
		t.Fatal(err)
	}
	receipt, err := p.UpgradeSubscriptionEPay(ctx, db, name, role)
	if err != nil {
		t.Fatal(err)
	}
	if receipt.Status != "committed" || receipt.PreservedDataSHA256 == "" || receipt.PreservedACLSHA256 == "" {
		t.Fatalf("missing preservation evidence: %+v", receipt)
	}
	replay, err := p.UpgradeSubscriptionEPay(ctx, db, name, role)
	if err != nil || replay.Status != "already_committed" || replay.PreservedDataSHA256 != receipt.PreservedDataSHA256 {
		t.Fatalf("replay: %+v %v", replay, err)
	}
	raw, _ := json.Marshal(receipt)
	t.Logf("SQL migration receipt: %s", raw)
	var original string
	if err = db.QueryRowContext(ctx, `SELECT snapshot->>'original' FROM payment_orders WHERE id='original-order'`).Scan(&original); err != nil || original != "preserve" {
		t.Fatalf("old data changed: %s %v", original, err)
	}
	if _, err = db.ExecContext(ctx, `SET ROLE `+role); err != nil {
		t.Fatal(err)
	}
	store, err := p.NewPostgres(ctx, db, name)
	if err != nil {
		t.Fatal(err)
	}
	e := p.Event{OrderID: "sub_" + strings.Repeat("a", 64), Type: p.Paid, Currency: "CNY", AmountMinor: 125, Evidence: p.Evidence{Provider: "epay", Merchant: "1001", Environment: "test", Transaction: "tx1", EventID: "event1", Payload: []byte("original-raw-input"), Signature: []byte("original-signature")}}
	entry, err := store.PutSubscriptionEvidence(ctx, e)
	if err != nil {
		t.Fatal(err)
	}
	// Different raw candidates never overwrite or own the same unverified identity.
	e.Evidence.Payload = []byte("equivalent-reordered-input")
	duplicate, err := store.PutSubscriptionEvidence(ctx, e)
	if err != nil || duplicate.ID == entry.ID || string(duplicate.Payload) != "equivalent-reordered-input" {
		t.Fatalf("original evidence replaced: %+v %v", duplicate, err)
	}
	if err = store.RetrySubscriptionEvidence(ctx, duplicate, p.ErrEvidence); err != nil {
		t.Fatal(err)
	}
	e.AmountMinor = 126
	if _, err = store.PutSubscriptionEvidence(ctx, e); err == nil {
		t.Fatal("changed payment meaning accepted")
	}
	reopened, err := p.NewPostgres(ctx, db, name)
	if err != nil {
		t.Fatal(err)
	}
	pending, err := reopened.PendingSubscriptionEvidence(ctx, 1)
	if err != nil || len(pending) != 1 || string(pending[0].Payload) != "original-raw-input" {
		t.Fatalf("restart read: %+v %v", pending, err)
	}
	for _, code := range []codes.Code{codes.PermissionDenied, codes.Unauthenticated, codes.NotFound, codes.InvalidArgument} {
		if err = reopened.RetrySubscriptionEvidence(ctx, entry, status.Error(code, "synthetic stage-local service error")); err != nil {
			t.Fatal(err)
		}
		var state string
		if err = db.QueryRowContext(ctx, `SELECT state FROM payment_subscription_inbox WHERE evidence_id=$1`, entry.ID).Scan(&state); err != nil || state != "pending" {
			t.Fatalf("generic status removed recoverable evidence: %s %q %v", code, state, err)
		}
	}
	if err = reopened.CompleteSubscriptionEvidence(ctx, entry, "sproof_synthetic_storage_receipt", 42); err != nil {
		t.Fatal(err)
	}
	if err = reopened.CompleteSubscriptionEvidence(ctx, entry, "sproof_synthetic_storage_receipt", 42); err != nil {
		t.Fatal(err)
	}
	if err = reopened.CompleteSubscriptionEvidence(ctx, entry, "sproof_changed_receipt", 43); err == nil {
		t.Fatal("changed grant receipt accepted")
	}
	pending, err = reopened.PendingSubscriptionEvidence(ctx, 1)
	if err != nil || len(pending) != 0 {
		t.Fatalf("settled row retried: %v %v", pending, err)
	}
	if _, err = db.ExecContext(ctx, `SELECT * FROM payment_subscription_upgrades`); err == nil {
		t.Fatal("runtime read owner-only upgrade receipt")
	}
	if _, err = db.ExecContext(ctx, `RESET ROLE`); err != nil {
		t.Fatal(err)
	}
	// Unknown predecessor shape is rejected rather than accepted by table presence.
	if _, err = db.ExecContext(ctx, `ALTER TABLE payment_subscription_inbox ADD COLUMN drift TEXT`); err != nil {
		t.Fatal(err)
	}
	if _, err = p.UpgradeSubscriptionEPay(ctx, db, name, role); err == nil {
		t.Fatal("catalog drift accepted")
	}
}
