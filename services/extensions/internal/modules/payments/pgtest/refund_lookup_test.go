// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package pgtest

import (
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

// Independent Go/PG acceptance. Core and provider are explicit test doubles;
// synthetic HMAC acceptance is NOT Rust verification or financial-ledger evidence.
const refundLookupSecret = "independent-local-refund-lookup-only"

func refundLookupDB(t *testing.T) (*sql.DB, string) {
	t.Helper()
	dsn := os.Getenv("PAYMENT_REFUND_LOOKUP_TEST_DSN")
	if dsn == "" {
		t.Skip("PAYMENT_REFUND_LOOKUP_TEST_DSN must identify an owned empty refund-lookup test database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	var name string
	if err = db.QueryRow("SELECT current_database()").Scan(&name); err != nil || !strings.HasPrefix(name, "lmm_refund_lookup_") || !strings.HasSuffix(name, "_test") {
		t.Fatal("refuse database outside the independent refund-lookup fixture", err)
	}
	return db, name
}

// Probe receipts survive the child restart, but live only in this disposable
// Go database. They deliberately do not resemble a real Core schema or ledger.
type refundLookupCore struct {
	p.Core
	db *sql.DB
}

func (c *refundLookupCore) Read(context.Context, string, p.Intent) error { return nil }
func (c *refundLookupCore) result(ctx context.Context, id int64, key, state string) (p.Receipt, error) {
	r := p.Receipt{ID: "probe_" + key, IntentID: id, Key: key, State: state, RequestHash: make([]byte, 32), JournalID: 1}
	raw, _ := json.Marshal(r)
	_, err := c.db.ExecContext(ctx, `INSERT INTO independent_refund_receipts(operation_key,intent_id,state,snapshot) VALUES($1,$2,$3,$4) ON CONFLICT(operation_key) DO NOTHING`, key, id, state, string(raw))
	if err != nil {
		return p.Receipt{}, err
	}
	return c.Receipt(ctx, id, key)
}
func (c *refundLookupCore) Receipt(ctx context.Context, id int64, key string) (p.Receipt, error) {
	var raw []byte
	err := c.db.QueryRowContext(ctx, `SELECT snapshot FROM independent_refund_receipts WHERE operation_key=$1 AND intent_id=$2`, key, id).Scan(&raw)
	if errors.Is(err, sql.ErrNoRows) {
		return p.Receipt{}, p.ErrNotFound
	}
	var r p.Receipt
	if err == nil {
		err = json.Unmarshal(raw, &r)
	}
	return r, err
}
func (c *refundLookupCore) Hold(ctx context.Context, _ string, id int64, key string, _ int64) (p.Receipt, error) {
	return c.result(ctx, id, key, p.RefundHeld)
}
func (c *refundLookupCore) BeginRefundDispatch(ctx context.Context, id int64, key, hold string) (p.Receipt, error) {
	if hold == "" {
		return p.Receipt{}, p.ErrConflict
	}
	return c.result(ctx, id, key, p.RefundHeld)
}
func refundLookupSignature(raw []byte) []byte {
	h := hmac.New(sha256.New, []byte(refundLookupSecret))
	_, _ = h.Write(raw)
	return h.Sum(nil)
}
func (c *refundLookupCore) AcceptProof(ctx context.Context, id int64, e p.Evidence) (p.ProofReceipt, error) {
	if e.Source != "webhook" || !hmac.Equal(e.Signature, refundLookupSignature(e.Payload)) {
		return p.ProofReceipt{}, p.ErrEvidence
	}
	h := sha256.Sum256(e.Payload)
	r := p.ProofReceipt{ID: "probe_proof_" + e.EventID, IntentID: id, EventID: e.EventID, PayloadSHA256: h[:], Payment: e, AcceptedUnixSeconds: time.Now().Unix()}
	raw, _ := json.Marshal(r)
	_, err := c.db.ExecContext(ctx, `INSERT INTO independent_refund_proofs(event_id,snapshot) VALUES($1,$2) ON CONFLICT(event_id) DO NOTHING`, e.EventID, string(raw))
	if err != nil {
		return p.ProofReceipt{}, err
	}
	return c.LookupProof(ctx, id, e)
}
func (c *refundLookupCore) LookupProof(ctx context.Context, id int64, e p.Evidence) (p.ProofReceipt, error) {
	var raw []byte
	err := c.db.QueryRowContext(ctx, `SELECT snapshot FROM independent_refund_proofs WHERE event_id=$1`, e.EventID).Scan(&raw)
	if errors.Is(err, sql.ErrNoRows) {
		return p.ProofReceipt{}, p.ErrNotFound
	}
	var r p.ProofReceipt
	if err == nil {
		err = json.Unmarshal(raw, &r)
	}
	if err == nil && r.IntentID != id {
		err = p.ErrConflict
	}
	return r, err
}
func (c *refundLookupCore) settle(ctx context.Context, id int64, key, hold string, e p.Evidence, state string) (p.Receipt, error) {
	proof, err := c.LookupProof(ctx, id, e)
	if err != nil || proof.ID != e.AcceptedProofID || hold == "" || !hmac.Equal(e.Signature, refundLookupSignature(e.Payload)) {
		return p.Receipt{}, p.ErrEvidence
	}
	// Count actual bridge invocations, not just unique simulated receipts.
	if _, err = c.db.ExecContext(ctx, `INSERT INTO independent_refund_calls(intent_id,state) VALUES($1,$2)`, id, state); err != nil {
		return p.Receipt{}, err
	}
	return c.result(ctx, id, key, state)
}
func (c *refundLookupCore) Commit(ctx context.Context, id int64, key, hold string, e p.Evidence) (p.Receipt, error) {
	return c.settle(ctx, id, key, hold, e, p.RefundCommitted)
}
func (c *refundLookupCore) Release(ctx context.Context, id int64, key, hold string, e p.Evidence) (p.Receipt, error) {
	return c.settle(ctx, id, key, hold, e, p.RefundReleased)
}

type refundLookupProvider struct {
	p.Adapter
	endpoint string
}

func (*refundLookupProvider) Channel() p.Channel {
	return p.Channel{ID: 1, Provider: "stripe", Merchant: "acct_independent", Environment: "test", Currency: "USD", Enabled: true}
}
func (*refundLookupProvider) RefundEnabled() bool               { return true }
func (*refundLookupProvider) PartialRefunds() bool              { return true }
func (*refundLookupProvider) ReplayWindow(string) time.Duration { return time.Hour }
func (a *refundLookupProvider) Refund(ctx context.Context, o p.Order, r p.Refund, key string) (p.RefundResult, error) {
	raw, _ := json.Marshal(struct {
		Order  p.Order
		Refund p.Refund
		Key    string
	}{o, r, key})
	req, _ := http.NewRequestWithContext(ctx, "POST", a.endpoint+"/refund", bytes.NewReader(raw))
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return p.RefundResult{}, err
	}
	defer res.Body.Close()
	var result p.RefundResult
	err = json.NewDecoder(res.Body).Decode(&result)
	return result, err
}
func (a *refundLookupProvider) Lookup(ctx context.Context, o p.Order) ([]p.Event, error) {
	q := url.Values{"order": {o.ID}, "intent_key": {o.Intent.Key}, "payment": {o.PaymentID}}
	for _, r := range o.Refunds {
		q.Set("refund", r.ID)
		q.Set("hold_key", r.HoldKey)
		q.Set("provider_refund", r.ProviderID)
	}
	req, _ := http.NewRequestWithContext(ctx, "GET", a.endpoint+"/lookup?"+q.Encode(), nil)
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer res.Body.Close()
	var events []p.Event
	err = json.NewDecoder(res.Body).Decode(&events)
	return events, err
}
func (*refundLookupProvider) Verify(req *http.Request, _ time.Time) (p.Event, error) {
	raw, err := io.ReadAll(req.Body)
	if err != nil {
		return p.Event{}, err
	}
	if req.Header.Get("X-Independent-Signature") != fmt.Sprintf("%x", refundLookupSignature(raw)) {
		return p.Event{}, p.ErrEvidence
	}
	var e p.Event
	if err = json.Unmarshal(raw, &e); err != nil {
		return e, err
	}
	e.Evidence.Source = "webhook"
	e.Evidence.Payload = raw
	e.Evidence.Signature = refundLookupSignature(raw)
	return e, nil
}

func refundLookupEvent(o p.Order, r p.Refund, kind, source string) p.Event {
	return p.Event{OrderID: o.ID, Type: kind, Currency: "USD", AmountMinor: r.AmountMinor, RefundID: "re_" + o.ID, LocalRefundID: r.ID, Evidence: p.Evidence{Provider: o.Intent.Provider, Merchant: o.Intent.Merchant, Environment: o.Intent.Environment, Transaction: o.PaymentID, EventID: "evt_" + source + "_" + o.ID, Source: source}}
}
func refundLookupJob(t *testing.T, o p.Order) p.Job {
	t.Helper()
	var found []p.Job
	for _, j := range o.Jobs {
		if j.Kind == "refund_lookup" {
			found = append(found, j)
		}
	}
	if len(found) != 1 {
		t.Fatalf("expected one persisted lookup, got %d", len(found))
	}
	return found[0]
}
func refundLookupOrder(t *testing.T, store *p.Postgres, id string) p.Order {
	t.Helper()
	o, err := store.Get(context.Background(), id)
	if err != nil {
		t.Fatal(err)
	}
	return o
}

// The submit child is killed by its parent after the durable job checkpoint.
// Later phases are different OS processes using the same PostgreSQL rows.
func TestIndependentRefundLookupChild(t *testing.T) {
	stage := os.Getenv("LMM_INDEPENDENT_REFUND_STAGE")
	if stage == "" {
		t.Skip("subprocess helper")
	}
	db, name := refundLookupDB(t)
	ctx := context.Background()
	store, err := p.NewPostgres(ctx, db, name)
	if err != nil {
		t.Fatal(err)
	}
	a := &refundLookupProvider{endpoint: os.Getenv("LMM_INDEPENDENT_REFUND_ENDPOINT")}
	s, err := p.New(store, &refundLookupCore{db: db}, a)
	if err != nil {
		t.Fatal(err)
	}
	id := os.Getenv("LMM_INDEPENDENT_REFUND_ORDER")
	switch stage {
	case "submit":
		if _, err = s.RequestRefund(ctx, "synthetic-session", id, "independent_refund_key_"+id, 300); err != nil {
			t.Fatal(err)
		}
		if err = s.Step(ctx, id); err != nil {
			t.Fatal(err)
		}
	case "work":
		for i := 0; i < 4; i++ {
			if err = s.Drain(ctx, 100); err != nil {
				t.Fatal(err)
			}
			if err = s.Step(ctx, id); err != nil {
				t.Fatal(err)
			}
		}
	case "callback":
		o := refundLookupOrder(t, store, id)
		var r p.Refund
		for _, candidate := range o.Refunds {
			r = candidate
		}
		kind := p.RefundSucceeded
		if id == "top_802" {
			kind = p.RefundFailed
		}
		e := refundLookupEvent(o, r, kind, "signed")
		raw, _ := json.Marshal(e)
		for i := 0; i < 2; i++ {
			req := httptest.NewRequest("POST", "/callback", bytes.NewReader(raw))
			req.Header.Set("X-Independent-Signature", fmt.Sprintf("%x", refundLookupSignature(raw)))
			if err = s.Receive(ctx, 1, req); err != nil {
				t.Fatal(err)
			}
			for n := 0; n < 4; n++ {
				if err = s.Drain(ctx, 100); err != nil {
					t.Fatal(err)
				}
				if err = s.Step(ctx, id); err != nil {
					t.Fatal(err)
				}
			}
		}
	default:
		t.Fatal("unknown child phase")
	}
	checkpoint := struct {
		PID   int
		Order p.Order
	}{os.Getpid(), refundLookupOrder(t, store, id)}
	raw, _ := json.Marshal(checkpoint)
	marker := os.Getenv("LMM_INDEPENDENT_REFUND_MARKER")
	if err = os.WriteFile(marker, raw, 0600); err != nil {
		t.Fatal(err)
	}
	if stage == "submit" {
		for {
			time.Sleep(time.Hour)
		}
	}
}

func TestIndependentRefundLookupPostgresProcessRestart(t *testing.T) {
	db, name := refundLookupDB(t)
	ctx, cancel := context.WithTimeout(context.Background(), 130*time.Second)
	defer cancel()
	// No DROP, shared tables, other databases, or implicit application setup.
	if _, err := db.ExecContext(ctx, p.Schema+`
CREATE TABLE independent_refund_receipts(operation_key text PRIMARY KEY,intent_id bigint NOT NULL,state text NOT NULL,snapshot jsonb NOT NULL);
CREATE TABLE independent_refund_proofs(event_id text PRIMARY KEY,snapshot jsonb NOT NULL);
CREATE TABLE independent_refund_calls(intent_id bigint NOT NULL,state text NOT NULL);`); err != nil {
		t.Fatal("requires fresh exclusively owned database", err)
	}
	store, err := p.NewPostgres(ctx, db, name)
	if err != nil {
		t.Fatal(err)
	}
	type call struct {
		method, order, key, hold, provider string
		at                                 time.Time
	}
	var mu sync.Mutex
	var calls []call
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.Method == "POST" && req.URL.Path == "/refund" {
			var body struct {
				Order  p.Order
				Refund p.Refund
				Key    string
			}
			if json.NewDecoder(req.Body).Decode(&body) != nil {
				w.WriteHeader(400)
				return
			}
			mu.Lock()
			calls = append(calls, call{"POST", body.Order.ID, body.Key, body.Refund.HoldKey, body.Refund.ProviderID, time.Now()})
			mu.Unlock()
			if body.Order.ID == "top_802" {
				conn, _, e := w.(http.Hijacker).Hijack()
				if e != nil {
					t.Error(e)
					return
				}
				_ = conn.Close()
				return
			}
			_ = json.NewEncoder(w).Encode(p.RefundResult{ID: "re_" + body.Order.ID, Status: "pending"})
			return
		}
		if req.Method == "GET" && req.URL.Path == "/lookup" {
			q := req.URL.Query()
			o, e := store.Get(req.Context(), q.Get("order"))
			if e != nil {
				w.WriteHeader(500)
				return
			}
			r := o.Refunds[q.Get("refund")]
			if q.Get("intent_key") != o.Intent.Key || q.Get("payment") != o.PaymentID || q.Get("hold_key") != r.HoldKey || q.Get("provider_refund") != r.ProviderID {
				t.Error("query lost original operation binding")
				w.WriteHeader(409)
				return
			}
			mu.Lock()
			calls = append(calls, call{"GET", o.ID, q.Get("intent_key"), r.HoldKey, r.ProviderID, time.Now()})
			mu.Unlock()
			_ = json.NewEncoder(w).Encode([]p.Event{refundLookupEvent(o, r, p.RefundSucceeded, "api")})
			return
		}
		w.WriteHeader(404)
	}))
	defer server.Close()
	count := func(method, id string) int {
		mu.Lock()
		defer mu.Unlock()
		n := 0
		for _, c := range calls {
			if c.method == method && c.order == id {
				n++
			}
		}
		return n
	}
	root := t.TempDir()
	childSequence := 0
	var pids []int
	child := func(stage, id string) {
		t.Helper()
		childSequence++
		marker := filepath.Join(root, fmt.Sprintf("child-%d.json", childSequence))
		cmd := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestIndependentRefundLookupChild$", "-test.v")
		cmd.Env = append(os.Environ(), "LMM_INDEPENDENT_REFUND_STAGE="+stage, "LMM_INDEPENDENT_REFUND_ORDER="+id, "LMM_INDEPENDENT_REFUND_ENDPOINT="+server.URL, "LMM_INDEPENDENT_REFUND_MARKER="+marker)
		var output bytes.Buffer
		cmd.Stdout = &output
		cmd.Stderr = &output
		if err := cmd.Start(); err != nil {
			t.Fatal(err)
		}
		pids = append(pids, cmd.Process.Pid)
		if stage == "submit" {
			deadline := time.Now().Add(15 * time.Second)
			for {
				if _, e := os.Stat(marker); e == nil {
					break
				}
				if time.Now().After(deadline) {
					_ = cmd.Process.Kill()
					_ = cmd.Wait()
					t.Fatalf("submit checkpoint timeout: %s", output.String())
				}
				time.Sleep(10 * time.Millisecond)
			}
			if err := cmd.Process.Kill(); err != nil {
				t.Fatal(err)
			}
			if err := cmd.Wait(); err == nil {
				t.Fatal("submit child was not killed")
			}
		} else if err := cmd.Wait(); err != nil {
			t.Fatalf("child %s failed: %v\n%s", stage, err, output.String())
		}
		var checkpoint struct {
			PID   int
			Order p.Order
		}
		raw, e := os.ReadFile(marker)
		if e != nil || json.Unmarshal(raw, &checkpoint) != nil || checkpoint.PID != cmd.Process.Pid || checkpoint.Order.ID != id {
			t.Fatal("child did not read original PG order", e)
		}
		t.Logf("phase=%s order=%s pid=%d", stage, id, checkpoint.PID)
	}
	ids := []string{"top_801", "top_802"}
	original := map[string]p.Order{}
	latest := time.Time{}
	for index, id := range ids {
		o := p.Order{ID: id, ChannelID: 1, Namespace: "stripe/acct_independent/test", Intent: p.Intent{ID: int64(801 + index), AccountID: 42, CreatedBy: 7, Provider: "stripe", Merchant: "acct_independent", Environment: "test", Currency: "USD", AmountMinor: 1000, Key: "independent_intent_key_" + id, RequestHash: make([]byte, 32)}, PaymentID: "pi_" + id, ProviderPaid: true, CreditState: "confirmed", CreditReceipt: "probe_original_credit", Refunds: map[string]p.Refund{}, Jobs: map[string]p.Job{}}
		if err = store.Insert(ctx, o); err != nil {
			t.Fatal(err)
		}
		before := time.Now()
		child("submit", id)
		after := time.Now()
		o = refundLookupOrder(t, store, id)
		j := refundLookupJob(t, o)
		if j.State != "pending" || j.Attempts != 0 || j.NextAttempt.Before(before.Add(time.Minute)) || j.NextAttempt.After(after.Add(time.Minute)) {
			t.Fatal("initial lookup is not scheduled exactly one minute after submit", j)
		}
		if j.NextAttempt.After(latest) {
			latest = j.NextAttempt
		}
		r := o.Refunds[j.RefundID]
		want := p.DispatchResponded
		if id == "top_802" {
			want = p.DispatchUnknown
		}
		if r.Dispatch != want || r.HoldReceipt == "" || r.DispatchReceipt == "" || r.HoldKey == "" || count("POST", id) != 1 {
			t.Fatal("original dispatch/hold missing", r)
		}
		mu.Lock()
		post := calls[len(calls)-1]
		mu.Unlock()
		if post.key == "" || o.Jobs[post.key].Kind != "refund_submit" || o.Jobs[post.key].RefundID != r.ID || post.hold != r.HoldKey {
			t.Fatal("provider POST did not use persisted original submit key")
		}
		original[id] = o
		child("work", id)
		if count("GET", id) != 0 || count("POST", id) != 1 {
			t.Fatal("restart polled before one minute or repeated POST")
		}
	}
	// Real monotonic waiting, no private clock override or altered due timestamp.
	if delay := time.Until(latest.Add(30 * time.Millisecond)); delay > 0 {
		timer := time.NewTimer(delay)
		select {
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			t.Fatal(ctx.Err())
		}
	}
	for _, id := range ids {
		before := time.Now()
		child("work", id)
		after := time.Now()
		o := refundLookupOrder(t, store, id)
		j := refundLookupJob(t, o)
		old := refundLookupJob(t, original[id])
		r := o.Refunds[j.RefundID]
		if count("GET", id) != 1 || count("POST", id) != 1 || j.Key != old.Key || j.RefundID != old.RefundID || j.State != "pending" || j.Attempts != 1 || r.HoldKey != original[id].Refunds[old.RefundID].HoldKey || r.HoldReceipt != original[id].Refunds[old.RefundID].HoldReceipt || o.Intent.Key != original[id].Intent.Key {
			t.Fatal("restart lost original task/key/hold or re-POSTed")
		}
		if j.NextAttempt.Before(before.Add(time.Minute)) || j.NextAttempt.After(after.Add(time.Minute)) {
			t.Fatal("observation retry is not scheduled exactly one minute later")
		}
		var settlements int
		if err = db.QueryRow(`SELECT count(*) FROM independent_refund_calls`).Scan(&settlements); err != nil || settlements != 0 {
			t.Fatal("unsigned observation reached simulated Core settlement", settlements, err)
		}
		if r.State == "committed" || r.State == "released" || r.State == "rejected" {
			t.Fatal("unsigned observation finalized original hold")
		}
		mu.Lock()
		for _, c := range calls {
			if c.method == "GET" && c.order == id && c.at.Before(old.NextAttempt) {
				t.Error("query happened before persisted one-minute deadline")
			}
		}
		mu.Unlock()
	}
	for _, id := range ids {
		child("callback", id)
		o := refundLookupOrder(t, store, id)
		j := refundLookupJob(t, o)
		r := o.Refunds[j.RefundID]
		want, state := "committed", p.RefundCommitted
		if id == "top_802" {
			want, state = "released", p.RefundReleased
		}
		if r.State != want {
			t.Errorf("trusted synthetic callback failed: %s got %s", id, r.State)
		}
		var n int
		if err = db.QueryRow(`SELECT count(*) FROM independent_refund_calls WHERE intent_id=$1 AND state=$2`, o.Intent.ID, state).Scan(&n); err != nil || n != 1 {
			t.Error("callback replay did not settle exactly once", n, err)
		}
	}
	// Direct persisted terminal fixtures isolate the stop-polling and projection
	// guards from callback completion; they do not prove Core rejection.
	for index, state := range []string{"committed", "released", "rejected"} {
		id := fmt.Sprintf("top_%d", 811+index)
		o := refundLookupOrder(t, store, ids[0])
		j := refundLookupJob(t, o)
		o.ID = id
		o.Intent.ID = int64(811 + index)
		o.PaymentID = "pi_" + id
		o.Intent.Key = "independent_intent_key_" + id
		for rid, r := range o.Refunds {
			r.State = state
			o.Refunds[rid] = r
		}
		j.State = "pending"
		j.NextAttempt = time.Now().Add(-time.Second)
		o.Jobs = map[string]p.Job{j.Key: j}
		if err = store.Insert(ctx, o); err != nil {
			t.Fatal(err)
		}
		child("work", id)
		o = refundLookupOrder(t, store, id)
		j = refundLookupJob(t, o)
		if j.State != "done" || o.Refunds[j.RefundID].State != state || count("GET", id) != 0 || count("POST", id) != 0 {
			t.Error("terminal refund continued provider polling", state)
		}
		if err = store.Mutate(ctx, id, func(o *p.Order) error {
			j := o.Jobs[j.Key]
			j.State = "review"
			j.ErrorCode = p.ErrUnsupported.Error()
			o.Jobs[j.Key] = j
			return nil
		}); err != nil {
			t.Fatal(err)
		}
		o = refundLookupOrder(t, store, id)
		want := "credited"
		if state == "committed" {
			want = "partially_refunded"
		}
		if o.State() != want {
			t.Errorf("terminal %s masked by lookup review: got %s want %s", state, o.State(), want)
		}
	}
	seen := map[int]bool{}
	for _, pid := range pids {
		if seen[pid] {
			t.Fatal("OS restart did not use a distinct child PID")
		}
		seen[pid] = true
	}
	t.Log("Executed scope: actual PostgreSQL + killed/reaped submit children + new OS workers + real minute deadlines + single HTTP POST per refund; Core/provider simulated, no Rust ledger or real merchant acceptance")
}
