package composition

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	_ "github.com/lib/pq"
	"os"
	"strings"
	"testing"
	"time"
)

type mailDenyAuthority struct{}

func (mailDenyAuthority) Check(context.Context, string, store.Account, store.Permission) (int64, error) {
	return 0, store.ErrForbidden
}
func TestStorePickupMailWorkerRealPG(t *testing.T) {
	path := os.Getenv("STORE_MAIL_FIXTURE")
	if path == "" {
		t.Skip("owned real PG/TLS SMTP fixture required")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var f struct {
		DSNFile string                 `json:"dsn_file"`
		SMTP    store.PickupSMTPConfig `json:"smtp"`
	}
	if json.Unmarshal(raw, &f) != nil {
		t.Fatal("decode fixture")
	}
	raw, err = os.ReadFile(f.DSNFile)
	if err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	repo, err := store.NewPostgres(db)
	if err != nil {
		t.Fatal(err)
	}
	s, err := store.New(repo, mailDenyAuthority{}, nil)
	if err != nil {
		t.Fatal(err)
	}
	c := Config{StorePickupMail: f.SMTP, HostCredential: []byte(strings.Repeat("h", 32))}
	if err = configureStorePickupMail(s, c); err != nil {
		t.Fatal(err)
	}
	// Reject identical bytes at distinct private paths; path checks alone are insufficient.
	key, err := os.ReadFile(f.SMTP.KeyFile)
	if err != nil {
		t.Fatal(err)
	}
	if configureStorePickupMail(s, c, key) == nil {
		t.Fatal("shared credential bytes accepted")
	}
	var id string
	if db.QueryRow(`SELECT body->>'order_id' FROM lmm_store.commands WHERE body->>'kind'='store-pickup-mail-v1' AND body->>'state'='pending'`).Scan(&id) != nil {
		t.Fatal("pending worker fixture absent")
	}
	state := func() string {
		t.Helper()
		var v string
		if db.QueryRow(`SELECT body->>'state' FROM lmm_store.commands WHERE body->>'order_id'=$1 AND body->>'kind'='store-pickup-mail-v1'`, id).Scan(&v) != nil {
			t.Fatal("query state")
		}
		return v
	}
	h, err := modules.NewHostWithLimit(nil, []byte(strings.Repeat("h", 32)), 1)
	if err != nil {
		t.Fatal(err)
	}
	if h.MarkReady() != nil {
		t.Fatal("ready")
	}
	release, err := h.TryAcquire()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	r := &Runtime{Store: s}
	defer r.Close()
	closed := false
	r.close = []func() error{func() error {
		select {
		case <-r.workerDone:
			closed = true
			return nil
		default:
			return errors.New("pool closed before worker joined")
		}
	}}
	r.StartWorkers(ctx, h)
	time.Sleep(5500 * time.Millisecond)
	if state() != "pending" {
		t.Fatal("worker bypassed admission")
	}
	release()
	end := time.Now().Add(6 * time.Second)
	for state() != "accepted_by_smtp" && time.Now().Before(end) {
		time.Sleep(20 * time.Millisecond)
	}
	if state() != "accepted_by_smtp" {
		t.Fatal("real runtime failed SMTP recovery")
	}
	h.BeginDrain()
	cancel()
	if err = r.Close(); err != nil {
		t.Fatal(err)
	}
	if !closed {
		t.Fatal("resource not closed")
	}
	if h.Wait(context.Background()) != nil {
		t.Fatal("permit leak")
	}
}
