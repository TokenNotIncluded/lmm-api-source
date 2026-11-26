package ssmcprelay

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type testRepo struct {
	relayPrice int64
	mu         sync.Mutex
	data       map[string]map[string][]byte
}
type testTx struct {
	store.Tx
	relayPrice int64
	data       map[string][]byte
}

func (r *testRepo) Within(_ context.Context, shop string, fn func(store.Tx) error) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	snapshot := map[string][]byte{}
	for k, v := range r.data[shop] {
		snapshot[k] = v
	}
	if err := fn(testTx{data: snapshot, relayPrice: r.relayPrice}); err != nil {
		return err
	}
	r.data[shop] = snapshot
	return nil
}
func (t testTx) RelayPricePerGiB() (int64, error) {
	if t.relayPrice == 0 {
		return store.DefaultRelayPricePerGiB, nil
	}
	return t.relayPrice, nil
}
func (t testTx) Get(kind, id string, v any) error {
	b, ok := t.data[kind+id]
	if !ok {
		return store.ErrNotFound
	}
	return json.Unmarshal(b, v)
}
func (t testTx) Put(kind, id string, v any) error {
	b, err := json.Marshal(v)
	if err == nil {
		t.data[kind+id] = b
	}
	return err
}

type testFunds struct {
	mu                  sync.Mutex
	ops                 map[string]store.Receipt
	losePay, loseSettle bool
}

func (f *testFunds) Available() bool { return true }
func (f *testFunds) Execute(_ context.Context, token string, c store.MoneyCommand) (store.Receipt, error) {
	if token != "user" && !(c.Kind == "refund" && token == "merchant") {
		return store.Receipt{}, store.ErrUnauthorized
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	r := f.record(c)
	if c.Kind == "pay" && f.losePay {
		f.losePay = false
		return store.Receipt{}, store.ErrPending
	}
	return r, nil
}
func (f *testFunds) record(c store.MoneyCommand) store.Receipt {
	if r, ok := f.ops[c.Key]; ok {
		return r
	}
	r := store.Receipt{Key: c.Key, CommandHash: store.ID(c), Reference: "core-" + c.Kind, Outcome: "succeeded"}
	f.ops[c.Key] = r
	return r
}
func (f *testFunds) Query(_ context.Context, _ string, c store.MoneyCommand) (store.Receipt, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if r, ok := f.ops[c.Key]; ok {
		return r, nil
	}
	return store.Receipt{}, store.ErrPending
}
func (f *testFunds) CompleteDelivery(_ context.Context, c store.MoneyCommand, payHash, delivery string) (store.Receipt, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	pay := f.ops[store.ID("store-money-v1", c.ShopID, c.OrderID, "pay")]
	if pay.CommandHash != payHash || len(delivery) != 64 || pay.Reference != c.PaymentRef {
		return store.Receipt{}, store.ErrConflict
	}
	r := f.record(c)
	if f.loseSettle {
		f.loseSettle = false
		return store.Receipt{}, store.ErrPending
	}
	return r, nil
}

type testRelay struct {
	mu            sync.Mutex
	grants        map[string]Entitlement
	lost, expired bool
}

func (r *testRelay) Quota(_ context.Context, id int64) (Quota, error) {
	return Quota{UserID: fmt.Sprint(id), Username: "test", Period: "2030-01", ResetAt: 1896134400, FreeBytes: "0", PaidBytes: "0", UsedBytes: "0", LegacyUsedBytes: "0", UploadedBytes: "0", DownloadedBytes: "0"}, nil
}
func (r *testRelay) Grant(_ context.Context, e Entitlement) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if prev, ok := r.grants[e.OrderID]; ok {
		if prev != e {
			return ErrGrantConflict
		}
		return nil
	}
	if r.expired {
		return ErrMonth
	}
	r.grants[e.OrderID] = e
	if r.lost {
		r.lost = false
		return ErrRelay
	}
	return nil
}
func fixture(t *testing.T) (*Service, *testRelay, *testFunds) {
	t.Helper()
	r := &testRelay{grants: map[string]Entitlement{}}
	f := &testFunds{ops: map[string]store.Receipt{}}
	s, err := New(r, &testRepo{data: map[string]map[string][]byte{}}, f, store.Account{Kind: "team", ID: 7})
	if err != nil {
		t.Fatal(err)
	}
	s.now = func() time.Time { return time.Unix(1894000000, 0) }
	return s, r, f
}

func TestPreviewConfirmReplayAndConcurrentSettlement(t *testing.T) {
	ctx := context.Background()
	s, r, f := fixture(t)
	o, err := s.Preview(ctx, 12, "request", 3)
	if err != nil || o.Bytes != "3221225472" || o.USDCents != 300 || o.State != "preview" || len(f.ops) != 0 || len(r.grants) != 0 {
		t.Fatal(o, err)
	}
	if other, e := s.Preview(ctx, 12, "request", 3); e != nil || other != o {
		t.Fatal(other, e)
	}
	if _, e := s.Preview(ctx, 12, "request", 4); !errors.Is(e, store.ErrConflict) {
		t.Fatal(e)
	}
	if _, e := s.Confirm(ctx, 13, "user", o.ID); !errors.Is(e, store.ErrNotFound) {
		t.Fatal(e)
	}
	if _, e := s.Confirm(ctx, 12, "", o.ID); !errors.Is(e, store.ErrUnauthorized) {
		t.Fatal(e)
	}
	f.losePay = true
	r.lost = true
	f.loseSettle = true
	for _, want := range []string{"payment_pending", "grant_pending", "settlement_pending"} {
		seen, e := s.Confirm(ctx, 12, "user", o.ID)
		if !errors.Is(e, store.ErrPending) || seen.State != want {
			t.Fatalf("%s %#v %v", want, seen, e)
		}
	}
	var group sync.WaitGroup
	for i := 0; i < 8; i++ {
		group.Add(1)
		go func() {
			defer group.Done()
			done, e := s.Confirm(ctx, 12, "user", o.ID)
			if e != nil || done.State != "completed" {
				t.Errorf("%#v %v", done, e)
			}
		}()
	}
	group.Wait()
	if len(f.ops) != 2 || len(r.grants) != 1 {
		t.Fatal(f.ops, r.grants)
	}
}

func TestNoNewChargeAfterExpiryAndExplicitRefund(t *testing.T) {
	ctx := context.Background()
	s, r, f := fixture(t)
	o, e := s.Preview(ctx, 2, "expired-before-reserve", 1)
	if e != nil {
		t.Fatal(e)
	}
	later, e := s.Preview(ctx, 2, "lost-before-expiry", 1)
	if e != nil {
		t.Fatal(e)
	}
	f.losePay = true
	if _, e = s.Confirm(ctx, 2, "user", later.ID); !errors.Is(e, store.ErrPending) {
		t.Fatal(e)
	}
	s.now = func() time.Time { return time.Unix(o.ResetAt, 0) }
	r.expired = true
	if expired, e := s.Confirm(ctx, 2, "user", o.ID); !errors.Is(e, ErrMonth) || expired.State != "expired" {
		t.Fatal(expired, e)
	}
	if expired, e := s.Confirm(ctx, 2, "user", later.ID); !errors.Is(e, ErrGrantConflict) || expired.State != "refund_required" {
		t.Fatal(expired, e)
	}
	if _, e = s.RefundExpired(ctx, 2, "user", o.ID); !errors.Is(e, store.ErrConflict) {
		t.Fatal(e)
	}
	for i := 0; i < 2; i++ {
		refund, e := s.RefundExpired(ctx, 2, "merchant", later.ID)
		if e != nil || refund.State != "refunded" {
			t.Fatal(refund, e)
		}
	}
	if len(f.ops) != 2 || len(r.grants) != 0 {
		t.Fatal(f.ops, r.grants)
	}
}

func TestPrivateAccountingHTTP(t *testing.T) {
	ctx := context.Background()
	key := strings.Repeat("ab", 32)
	mode := "ok"
	restore := ""
	grant := Entitlement{"5", "order", "2030-01", "1073741824"}
	second := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { t.Error("followed redirect with private request") }))
	defer second.Close()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.Header.Get("Authorization") != "Bearer "+key || r.Header.Get("Content-Type") != "application/json" {
			t.Error("missing service authentication")
		}
		if mode == "redirect" {
			http.Redirect(w, r, second.URL, 307)
			return
		}
		if mode == "month" {
			w.WriteHeader(409)
			fmt.Fprint(w, `{"error":"period_mismatch"}`)
			return
		}
		switch r.URL.Path {
		case "/internal/v1/ssmcp/quota":
			var request map[string]string
			_ = json.NewDecoder(r.Body).Decode(&request)
			if request["user_id"] != "5" {
				t.Error(request)
			}
			id := "5"
			if mode == "wrong" {
				id = "6"
			}
			fmt.Fprintf(w, `{"user_id":%q,"username":"alice","period":"2030-01","utc_offset_minutes":480,"reset_at":1896134400,"free_bytes":"100","paid_bytes":"0","used_bytes":"8","uploaded_bytes":"3","downloaded_bytes":"8","legacy_used_bytes":"0","limit_bytes":"100","remaining_bytes":"92"}`, id)
		case "/internal/v1/ssmcp/entitlements":
			var in Entitlement
			_ = json.NewDecoder(r.Body).Decode(&in)
			if in != grant {
				t.Error(in)
			}
			if mode == "wrong" {
				in.Bytes = "1"
			}
			_ = json.NewEncoder(w).Encode(struct {
				Entitlement
				Status string `json:"status"`
			}{in, restore})
		default:
			t.Error(r.URL.Path)
		}
	}))
	defer srv.Close()
	if _, e := NewClient("http://relay.example", key); e == nil {
		t.Fatal("public cleartext")
	}
	c, e := NewClient(srv.URL, key)
	if e != nil {
		t.Fatal(e)
	}
	q, e := c.Quota(ctx, 5)
	if e != nil || q.UsedBytes != "8" {
		t.Fatal(q, e)
	}
	mode = "wrong"
	if _, e = c.Quota(ctx, 5); !errors.Is(e, ErrRelay) {
		t.Fatal(e)
	}
	mode = "ok"
	for _, restore = range []string{"granted", "replayed"} {
		if e = c.Grant(ctx, grant); e != nil {
			t.Fatal(e)
		}
	}
	mode = "wrong"
	if e = c.Grant(ctx, grant); !errors.Is(e, ErrRelay) {
		t.Fatal(e)
	}
	mode = "month"
	if e = c.Grant(ctx, grant); !errors.Is(e, ErrMonth) {
		t.Fatal(e)
	}
	mode = "redirect"
	if _, e = c.Quota(ctx, 5); !errors.Is(e, ErrRelay) {
		t.Fatal(e)
	}
}

func TestFrozenConfigurablePriceUsesOriginalCreditsAndBytes(t *testing.T) {
	ctx := context.Background()
	svc, relay, f := fixture(t)
	repo := svc.repo.(*testRepo)
	repo.relayPrice = 3
	first, e := svc.Preview(ctx, 12, "tiny", 3)
	if e != nil || first.PricingVersion != 1 || first.TotalPriceQuota != 9 || first.UnitPriceQuotaPerGiB != 3 || first.USDCents != 0 || first.Bytes != "3221225472" {
		t.Fatal(first, e)
	}
	repo.relayPrice = 800000
	same, e := svc.Preview(ctx, 12, "tiny", 3)
	if e != nil || same != first {
		t.Fatal(same, e)
	}
	second, e := svc.Preview(ctx, 12, "new", 3)
	if e != nil || second.UnitPriceQuotaPerGiB != 800000 || second.TotalPriceQuota != 2400000 || second.USDCents != 480 || second.Bytes != first.Bytes {
		t.Fatal(second, e)
	}
	f.losePay = true
	if _, e := svc.Confirm(ctx, 12, "user", first.ID); !errors.Is(e, store.ErrPending) {
		t.Fatal(e)
	}
	repo.relayPrice = 500000
	settled, e := svc.Confirm(ctx, 12, "user", first.ID)
	if e != nil || settled.State != "completed" {
		t.Fatal(settled, e)
	}
	for _, kind := range []string{"pay", "settle"} {
		cmd := settled.command(kind)
		if cmd.Total != (store.Price{Amount: 9, Unit: "credits"}) || cmd.OrderSnapshot.UnitPrice != (store.Price{Amount: 3, Unit: "credits"}) || cmd.OrderSnapshot.Quantity != 3 || f.ops[cmd.Key].CommandHash != store.ID(cmd) {
			t.Fatalf("%s changed the original quote", kind)
		}
	}
	if len(f.ops) != 2 || len(relay.grants) != 1 || relay.grants[first.ID].Bytes != first.Bytes {
		t.Fatal("duplicate debit or incorrect byte grant")
	}
	repo.relayPrice = maxQuoteCredits
	if _, e := svc.Preview(ctx, 12, "exact", 1); e != nil {
		t.Fatal(e)
	}
	if _, e := svc.Preview(ctx, 12, "unsafe", 2); !errors.Is(e, store.ErrInvalid) {
		t.Fatalf("unsafe total: %v", e)
	}
	if _, e := svc.Preview(ctx, 12, "unsafe-bytes", 17179869184); !errors.Is(e, store.ErrInvalid) {
		t.Fatalf("unsafe bytes: %v", e)
	}
}

func TestExistingUSDReceiptRemainsReplayableAfterChangingTariff(t *testing.T) {
	ctx := context.Background()
	svc, r, f := fixture(t)
	repo := svc.repo.(*testRepo)
	id := store.ID("ssmcp-relay-order-v1", int64(12), "historical")
	old := Order{ID: id, UserID: 12, Seller: svc.seller, Period: "2030-01", ResetAt: 1896134400, GiB: 2, Bytes: "2147483648", USDCents: 200, State: "payment_pending"}
	if err := repo.Within(ctx, namespace(12), func(tx store.Tx) error { return tx.Put("commands", id, old) }); err != nil {
		t.Fatal(err)
	}
	historicalCommand := old.command("pay")
	if historicalCommand.Total != (store.Price{Amount: 200, Currency: "USD"}) || historicalCommand.OrderSnapshot.UnitPrice != (store.Price{Amount: 100, Currency: "USD"}) {
		t.Fatal("legacy command unit changed")
	}
	originalReceipt, e := f.Execute(ctx, "user", historicalCommand)
	if e != nil {
		t.Fatal(e)
	}
	repo.relayPrice = 1
	o, e := svc.Preview(ctx, 12, "historical", 2)
	if e != nil || o.TotalPriceQuota != 1000000 || o.UnitPriceQuotaPerGiB != 500000 || o.PricingVersion != 0 {
		t.Fatal(o, e)
	}
	settled, e := svc.Confirm(ctx, 12, "user", id)
	if e != nil || settled.State != "completed" || len(r.grants) != 1 || len(f.ops) != 2 || f.ops[historicalCommand.Key] != originalReceipt {
		t.Fatal(settled, e)
	}
	if got := settled.command("pay"); store.ID(got) != store.ID(historicalCommand) {
		t.Fatal("old command hash changed")
	}
}
