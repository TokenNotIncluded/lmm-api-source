package payments_test

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments/rustbridge"
)

// This executable is a real Go module worker process, not lmm-extensions main.
// Only the Stripe transport destination changes; the production parser, storage,
// state machine and Rust bridge are used unchanged. No webhook secret enters Go.
type refundProcessTransport struct {
	base   *url.URL
	client *http.Client
}

func (tr refundProcessTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	clone := request.Clone(request.Context())
	target := *request.URL
	target.Scheme, target.Host = tr.base.Scheme, tr.base.Host
	clone.URL = &target
	clone.Host = tr.base.Host
	return tr.client.Do(clone)
}

type refundProcessFixture struct {
	proofJointFixture
	ProviderURL  string `json:"provider_url"`
	MarkerDir    string `json:"marker_dir"`
	EvidenceFile string `json:"evidence_file"`
}
type refundSignedEvent struct {
	Payload   string `json:"payload"`
	Signature string `json:"signature"`
}
type refundRecoveryEvidence struct {
	Created       refundSignedEvent `json:"created"`
	Failed        refundSignedEvent `json:"failed"`
	LateSuccess   refundSignedEvent `json:"late_success"`
	ConflictingID refundSignedEvent `json:"conflicting_id"`
}

func refundPrivateJSON(t *testing.T, path string, value any) {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal("cannot encode private receipt")
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal("private receipt already exists or cannot be created")
	}
	if _, err = file.Write(raw); err != nil {
		t.Fatal("cannot write receipt")
	}
	if err = file.Sync(); err != nil {
		t.Fatal("cannot sync receipt")
	}
	if err = file.Close(); err != nil {
		t.Fatal("cannot close receipt")
	}
}
func refundEvidence(f refundProcessFixture, event refundSignedEvent) p.Evidence {
	var parsed struct {
		ID string `json:"id"`
	}
	if json.Unmarshal([]byte(event.Payload), &parsed) != nil {
		panic("invalid synthetic event")
	}
	return p.Evidence{Provider: f.Channel.Provider, Merchant: f.Channel.Merchant, Environment: f.Channel.Environment, Transaction: "pi_" + strconv.FormatInt(f.IntentID, 10), EventID: parsed.ID, Payload: []byte(event.Payload), Signature: []byte(event.Signature), Source: "webhook"}
}

func TestRefundProcessWorkerFixture(t *testing.T) {
	path, phase := os.Getenv("LMM_REFUND_PROCESS_FIXTURE"), os.Getenv("LMM_REFUND_PROCESS_PHASE")
	if path == "" {
		t.Skip("isolated refund process orchestrator required; NOT RUN")
	}
	if phase != "dispatch" && phase != "recover" && phase != "capacity" {
		t.Fatal("invalid refund process phase")
	}
	info, err := os.Stat(path)
	if err != nil || info.Mode().Perm()&0077 != 0 {
		t.Fatal("fixture must be private")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("cannot read fixture")
	}
	var f refundProcessFixture
	if json.Unmarshal(raw, &f) != nil || f.IntentID <= 0 || f.Account <= 0 || f.Channel.Environment != "test" || len(f.Events) == 0 {
		t.Fatal("invalid synthetic fixture")
	}
	u, err := url.Parse(f.ProviderURL)
	if err != nil || u.Scheme != "http" || u.Hostname() != "127.0.0.1" || u.User != nil {
		t.Fatal("provider must be owned loopback TCP fixture")
	}
	info, err = os.Stat(f.DatabaseURLFile)
	if err != nil || info.Mode().Perm()&0077 != 0 {
		t.Fatal("Go database URL must be private")
	}
	dsn, err := os.ReadFile(f.DatabaseURLFile)
	if err != nil {
		t.Fatal("cannot read isolated Go database reference")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	db, err := sql.Open("postgres", strings.TrimSpace(string(dsn)))
	if err != nil {
		t.Fatal("cannot open isolated Go database")
	}
	defer db.Close()
	db.SetMaxOpenConns(4)
	store, err := p.NewPostgres(ctx, db, f.DatabaseName)
	if err != nil {
		t.Fatal("Go isolation guard rejected fixture")
	}
	client, err := coreclient.New(f.Socket, []byte(f.Token))
	if err != nil {
		t.Fatal("cannot connect real Core")
	}
	defer client.Close()
	bridge, err := rustbridge.New(client)
	if err != nil {
		t.Fatal(err)
	}
	stripe, err := p.NewStripe(p.StripeConfig{Channel: f.Channel, APIKey: "sk_test_refund_synthetic_only", APIVersion: "refund-synthetic-version", SuccessURL: "https://refund.example.invalid/paid", CancelURL: "https://refund.example.invalid/cancel"})
	if err != nil {
		t.Fatal(err)
	}
	p.ReplaceStripeTransportForTest(stripe, refundProcessTransport{base: u, client: &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 12 * time.Second}})
	service, err := p.New(store, bridge, stripe)
	if err != nil {
		t.Fatal(err)
	}
	callback := httptest.NewServer(service.WebhookHandler())
	defer callback.Close()
	deliver := func(event refundSignedEvent) {
		req, err := http.NewRequestWithContext(ctx, "POST", callback.URL+"/callbacks/1", strings.NewReader(event.Payload))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Stripe-Signature", event.Signature)
		response, err := callback.Client().Do(req)
		if err != nil {
			t.Fatal("callback loopback failed")
		}
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		if response.StatusCode != http.StatusOK {
			t.Fatalf("Core-backed callback status %d", response.StatusCode)
		}
	}
	orderID := "top_" + strconv.FormatInt(f.IntentID, 10)
	if phase == "capacity" {
		runCapacityRecovery(t, ctx, f, store, service, deliver)
		return
	}
	if phase == "dispatch" {
		_, err = service.Create(ctx, f.User, p.PrepareRequest{Key: f.PrepareKey, AccountID: f.Account, ChannelID: 1, Currency: "USD", AmountMinor: 100})
		if err != nil {
			t.Fatal("original Core prepare mismatch")
		}
		deliver(refundSignedEvent{Payload: f.Events[0].Payload, Signature: f.Events[0].Signature})
		for i := 0; i < 3; i++ {
			if err = service.Work(ctx, 100); err != nil {
				t.Fatal(err)
			}
		}
		order, err := store.Get(ctx, orderID)
		if err != nil || !order.Credited() {
			t.Fatal("real Core credit did not settle")
		}
		refund, err := service.RequestRefund(ctx, f.User, orderID, "refund_real_os_0001", 40)
		if err != nil || refund.HoldReceipt == "" {
			t.Fatal("real Core reservation missing")
		}
		refundPrivateJSON(t, filepath.Join(f.MarkerDir, "held.json"), map[string]any{"pid": os.Getpid(), "refund": refund})
		// The owned TCP provider pauses after receiving the POST. The orchestrator
		// must kill this OS process; no object reconstruction is used as a restart.
		if err = service.Work(ctx, 100); err != nil {
			t.Fatal(err)
		}
		t.Fatal("orchestrator failed to stop the worker at provider response loss")
	}
	var original p.Refund
	var submitKey string
	var unknownJob p.Job
	for {
		if err = service.Work(ctx, 100); err != nil {
			t.Fatal(err)
		}
		order, err := store.Get(ctx, orderID)
		if err != nil {
			t.Fatal(err)
		}
		for key, job := range order.Jobs {
			if job.Kind == "refund_submit" && job.State == "review" {
				original = order.Refunds[job.RefundID]
				submitKey = key
				unknownJob = job
			}
		}
		if submitKey != "" {
			break
		}
		select {
		case <-ctx.Done():
			t.Fatal("original lease did not recover")
		case <-time.After(100 * time.Millisecond):
		}
	}
	if unknownJob.Attempts != 2 || unknownJob.ErrorCode != p.ErrUnsupported.Error() {
		t.Fatal("UNKNOWN did not stop the second provider attempt before IO")
	}
	if original.Dispatch != p.DispatchUnknown || original.State == "released" || original.DispatchReceipt == "" {
		t.Fatal("unknown dispatch or Core registration was lost")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "unknown.json"), map[string]any{"pid": os.Getpid(), "refund": original, "submit_key": submitKey, "job": unknownJob})
	var evidence refundRecoveryEvidence
	for {
		raw, err = os.ReadFile(f.EvidenceFile)
		if err == nil {
			if json.Unmarshal(raw, &evidence) != nil {
				t.Fatal("invalid immutable signed evidence")
			}
			break
		}
		if !os.IsNotExist(err) {
			t.Fatal("cannot read evidence gate")
		}
		select {
		case <-ctx.Done():
			t.Fatal("evidence gate deadline")
		case <-time.After(50 * time.Millisecond):
		}
	}
	for i := 0; i < 2; i++ {
		deliver(evidence.Created)
		deliver(evidence.Failed)
		for j := 0; j < 3; j++ {
			if err = service.Work(ctx, 100); err != nil {
				t.Fatal(err)
			}
		}
	}
	order, err := store.Get(ctx, orderID)
	if err != nil || order.Refunds[original.ID].State != "released" {
		t.Fatal("bound failure did not release real Core hold")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "released.json"), map[string]any{
		"pid":    os.Getpid(),
		"refund": order.Refunds[original.ID],
	})
	for {
		if _, err = os.Stat(filepath.Join(f.MarkerDir, "late-success.ready")); err == nil {
			break
		} else if !os.IsNotExist(err) {
			t.Fatal("cannot read late-success gate")
		}
		select {
		case <-ctx.Done():
			t.Fatal("late-success gate deadline")
		case <-time.After(50 * time.Millisecond):
		}
	}
	// A fresh valid signature does not make a changed payload for an existing
	// event ID valid. A late provider success follows the real webhook route.
	conflicting := refundEvidence(f, evidence.ConflictingID)
	if _, err = bridge.AcceptProof(ctx, f.IntentID, conflicting); !errors.Is(err, p.ErrConflict) {
		t.Fatalf("conflicting signed event ID accepted: %v", err)
	}
	deliver(evidence.LateSuccess)
	// Seed the old handler's durable done inbox without a settlement job.
	// Replaying the same signed webhook must repair it through the real worker.
	lateID := refundEvidence(f, evidence.LateSuccess).EventID
	pending, err := store.Events(ctx, time.Now(), 100)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, event := range pending {
		if event.Evidence.EventID == lateID {
			if err = store.FinishEvent(ctx, event.Key, "done", "", time.Now()); err != nil {
				t.Fatal(err)
			}
			found = true
		}
	}
	if !found {
		t.Fatal("accepted late success not present in durable inbox")
	}
	if err = store.Mutate(ctx, orderID, func(order *p.Order) error {
		order.ErrorCode = "conflicting_refund_terminal_state"
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	deliver(evidence.LateSuccess)
	for i := 0; i < 3; i++ {
		if err = service.Work(ctx, 100); err != nil {
			t.Fatal(err)
		}
	}
	order, err = store.Get(ctx, orderID)
	if err != nil || order.Refunds[original.ID].State != "committed" || order.ErrorCode != "" {
		t.Fatal("late success did not commit through the Go worker and Core")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "settled.json"), map[string]any{
		"pid":                             os.Getpid(),
		"refund":                          order.Refunds[original.ID],
		"submit_key":                      submitKey,
		"duplicate_events_checked":        true,
		"conflicting_event_id_denied":     true,
		"success_after_release_committed": true,
		"legacy_done_success_replayed":    true,
	})
	// Keep the actual executed image alive for parent FD/PID attestation.
	for {
		select {
		case <-ctx.Done():
			t.Fatal("orchestrator did not reap settled worker")
		case <-time.After(50 * time.Millisecond):
		}
	}
}

// This additional case keeps the original response-loss case and its exact
// assertions intact. Both competing refunds use the real module worker and Core.
func runCapacityRecovery(t *testing.T, ctx context.Context, f refundProcessFixture, store *p.Postgres, service *p.Service, deliver func(refundSignedEvent)) {
	t.Helper()
	orderID := "top_" + strconv.FormatInt(f.IntentID, 10)
	work := func() {
		for i := 0; i < 3; i++ {
			if err := service.Work(ctx, 100); err != nil {
				t.Fatal(err)
			}
		}
	}
	wait := func(name string) {
		for {
			if _, err := os.Stat(filepath.Join(f.MarkerDir, name)); err == nil {
				return
			} else if !os.IsNotExist(err) {
				t.Fatal(err)
			}
			select {
			case <-ctx.Done():
				t.Fatal("capacity checkpoint deadline")
			case <-time.After(50 * time.Millisecond):
			}
		}
	}
	evidence := func(n string) refundRecoveryEvidence {
		raw, err := os.ReadFile(filepath.Join(f.MarkerDir, "capacity-post-"+n+".json"))
		if err != nil {
			t.Fatal(err)
		}
		var e refundRecoveryEvidence
		if json.Unmarshal(raw, &e) != nil {
			t.Fatal("invalid capacity evidence")
		}
		return e
	}
	if _, err := service.Create(ctx, f.User, p.PrepareRequest{Key: f.PrepareKey, AccountID: f.Account, ChannelID: 1, Currency: "USD", AmountMinor: 100}); err != nil {
		t.Fatal(err)
	}
	deliver(refundSignedEvent{Payload: f.Events[0].Payload, Signature: f.Events[0].Signature})
	work()
	old, err := service.RequestRefund(ctx, f.User, orderID, "capacity_old_refund_001", 40)
	if err != nil || old.HoldReceipt == "" {
		t.Fatal("old hold missing", err)
	}
	work()
	a := evidence("1")
	deliver(a.Created)
	deliver(a.Failed)
	work()
	order, err := store.Get(ctx, orderID)
	if err != nil || order.Refunds[old.ID].State != "released" {
		t.Fatal("old failure did not release", err)
	}
	retry, err := service.RequestRefund(ctx, f.User, orderID, "capacity_new_refund_001", 100)
	if err != nil || retry.HoldReceipt == "" {
		t.Fatal("retry did not obtain full capacity", err)
	}
	work()
	b := evidence("2")
	deliver(b.Created)
	work()
	deliver(a.LateSuccess)
	work()
	findReview := func() p.Job {
		order, err := store.Get(ctx, orderID)
		if err != nil {
			t.Fatal(err)
		}
		if order.Refunds[old.ID].State != "released" || order.Refunds[retry.ID].State == "released" {
			t.Fatal("capacity conflict changed either hold")
		}
		for _, job := range order.Jobs {
			if job.Kind == "commit" && job.RefundID == old.ID && job.State == "review" && job.ErrorCode == p.ErrConflict.Error() && job.Evidence.AcceptedProofID != "" {
				return job
			}
		}
		t.Fatal("capacity conflict not durably reviewable")
		return p.Job{}
	}
	original := findReview()
	// Reconciliation while B is still pending must fail closed with the same key.
	if err := service.Reconcile(ctx, orderID); err != nil {
		t.Fatal(err)
	}
	work()
	if again := findReview(); again.Key != original.Key || again.Evidence.AcceptedProofID != original.Evidence.AcceptedProofID {
		t.Fatal("reconciliation changed the original identity")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "capacity-blocked.json"), map[string]any{"pid": os.Getpid(), "old": old, "retry": retry, "job": original})
	wait("capacity-fail-retry.ready")
	deliver(b.Failed)
	if err := service.Work(ctx, 1); err != nil {
		t.Fatal(err)
	}
	order, err = store.Get(ctx, orderID)
	if err != nil || order.Refunds[retry.ID].State != "released" {
		t.Fatal("bound retry failure did not release", err)
	}
	queued := order.Jobs[original.Key]
	if queued.State != "pending" || queued.Evidence.AcceptedProofID != original.Evidence.AcceptedProofID {
		t.Fatal("confirmed release did not durably resume the original settlement")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "capacity-freed.json"), map[string]any{"pid": os.Getpid()})
	wait("capacity-spent.ready")
	// Production host scheduling invokes Work, without a manual Reconcile call.
	work()
	// Replays and another trusted reconciliation must not create another debit.
	deliver(a.LateSuccess)
	work()
	if err := service.Reconcile(ctx, orderID); err != nil {
		t.Fatal(err)
	}
	work()
	order, err = store.Get(ctx, orderID)
	if err != nil || order.Refunds[old.ID].State != "committed" || order.Refunds[retry.ID].State != "released" {
		t.Fatal("capacity recovery did not settle exact original refund", err)
	}
	final := order.Jobs[original.Key]
	if final.State != "done" || final.Receipt == nil || final.Evidence.AcceptedProofID != original.Evidence.AcceptedProofID {
		t.Fatal("recovered receipt or original proof lost")
	}
	refundPrivateJSON(t, filepath.Join(f.MarkerDir, "capacity-settled.json"), map[string]any{"pid": os.Getpid(), "same_key": final.Key == original.Key, "same_proof": true, "replay_checked": true})
	for {
		select {
		case <-ctx.Done():
			t.Fatal("orchestrator did not reap capacity worker")
		case <-time.After(50 * time.Millisecond):
		}
	}
}
