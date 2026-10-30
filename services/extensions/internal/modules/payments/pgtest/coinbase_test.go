package pgtest

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"database/sql"
	"encoding/pem"
	"os"
	"regexp"
	"sync"
	"testing"
	"time"

	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

// This fixture tests the newly generated outbound key's PostgreSQL durability.
// It grants no money and does not stand in for Core's signature/ledger tests.
type coinbasePreparedCore struct{ p.Core }

func (coinbasePreparedCore) Prepare(_ context.Context, credential string, r p.PrepareRequest) (p.Intent, error) {
	if credential != "synthetic_authorized_user" {
		return p.Intent{}, p.ErrDenied
	}
	return p.Intent{ID: 1, AccountID: r.AccountID, CreatedBy: 7, Provider: "coinbase", Merchant: "merchant_fixture", Environment: "test", Currency: r.Currency,
		AmountMinor: r.AmountMinor, CreditUnits: r.AmountMinor * 5000, CreditUnit: "credit_500k_usd", Key: r.Key, RequestHash: make([]byte, 32)}, nil
}

func TestCoinbasePostgresRetainsFirstUUIDAcrossConcurrentCreates(t *testing.T) {
	dsn := os.Getenv("PAYMENT_COINBASE_TEST_DSN")
	if dsn == "" {
		t.Skip("PAYMENT_COINBASE_TEST_DSN must name a fresh disposable lmm_payments_test database")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(12)
	var name string
	if err = db.QueryRowContext(ctx, "SELECT current_database()").Scan(&name); err != nil || name != "lmm_payments_test" {
		t.Fatal("refuse non-fixture database", name, err)
	}
	// Require a fresh isolated fixture. Never drop or reset an existing schema.
	if _, err = db.ExecContext(ctx, p.Schema); err != nil {
		t.Fatal(err)
	}
	store, err := p.NewPostgres(ctx, db, name)
	if err != nil {
		t.Fatal(err)
	}
	private, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalECPrivateKey(private)
	if err != nil {
		t.Fatal(err)
	}
	adapter, err := p.NewCoinbase(p.CoinbaseConfig{
		Channel: p.Channel{ID: 8, Provider: "coinbase", Merchant: "merchant_fixture", Environment: "test", Currency: "USD", Enabled: true},
		APIKey:  "organizations/fixture/apiKeys/fixture", PrivateKey: string(pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: der})),
		SuccessURL: "https://example.test/wallet", CancelURL: "https://example.test/wallet",
	})
	if err != nil {
		t.Fatal(err)
	}
	svc, err := p.New(store, coinbasePreparedCore{}, adapter)
	if err != nil {
		t.Fatal(err)
	}
	request := p.PrepareRequest{Key: "coinbase_postgres_request_001", AccountID: 42, ChannelID: 8, Currency: "USD", AmountMinor: 1000}
	var group sync.WaitGroup
	for range 12 {
		group.Go(func() {
			if _, err := svc.Create(ctx, "synthetic_authorized_user", request); err != nil {
				t.Error(err)
			}
		})
	}
	group.Wait()
	before, err := store.Get(ctx, "top_1")
	if err != nil || len(before.Jobs) != 1 {
		t.Fatal("concurrent create did not retain one checkout job", before, err)
	}
	var key string
	for id, job := range before.Jobs {
		key = id
		if job.Key != id || job.Kind != "checkout" || job.Attempts != 0 {
			t.Fatal("provider I/O occurred before durable job", job)
		}
	}
	if !regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`).MatchString(key) {
		t.Fatal("provider key is not a persisted UUIDv4", key)
	}
	if err = db.Close(); err != nil {
		t.Fatal(err)
	}
	reopened, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	store, err = p.NewPostgres(ctx, reopened, name)
	if err != nil {
		t.Fatal(err)
	}
	svc, err = p.New(store, coinbasePreparedCore{}, adapter)
	if err != nil {
		t.Fatal(err)
	}
	after, err := svc.Create(ctx, "synthetic_authorized_user", request)
	if err != nil || len(after.Jobs) != 1 || after.Jobs[key].Key != key || after.Credited() {
		t.Fatal("reconnect/replay changed original checkout identity", after, err)
	}
	var orders, events int
	if err = reopened.QueryRowContext(ctx, "SELECT (SELECT count(*) FROM payment_orders),(SELECT count(*) FROM payment_events)").Scan(&orders, &events); err != nil || orders != 1 || events != 0 {
		t.Fatal("unexpected persistent side effect", orders, events, err)
	}
}
