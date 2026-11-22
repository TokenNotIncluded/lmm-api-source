package assistant_services

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

func invitationService(t *testing.T, db *sql.DB, proof *pgServiceConfirm, sender InvitationSender, user int64) *Service {
	t.Helper()
	s, e := New(context.Background(), db, pgServiceAuthority{}, Dependencies{Confirmations: proof, Invitations: &invitationTestReader{facts: invitationTestFacts(user)}, InvitationMail: sender})
	if e != nil {
		t.Fatal("restricted invitation service unavailable", e)
	}
	return s
}
func invitationPrepare(t *testing.T, s *Service, credential, label string) (assistant.AgentInvocation, string) {
	t.Helper()
	in := pgServiceInvocation(credential, "send_invitation", `{"email":"visitor+tag@example.invalid"}`)
	if label != "" {
		in.Call.OperationID = label
	}
	out := pgServiceMust(t, s, in)
	if out.State != assistant.AgentToolPending {
		t.Fatal("model call sent or bypassed confirmation")
	}
	return in, pgServiceConfirmationID(t, out)
}
func invitationResultStatus(t *testing.T, raw json.RawMessage, want string) {
	t.Helper()
	var got map[string]any
	if json.Unmarshal(raw, &got) != nil {
		t.Fatal("invalid receipt")
	}
	if got["status"] != want || got["retry_safe"] != false || got["delivery_confirmed"] != false {
		t.Fatal("incorrect durable outcome", string(raw))
	}
	if (got["smtp_accepted"] == true) != (want == "accepted_by_smtp") {
		t.Fatal("delivery falsely accepted", string(raw))
	}
}
func TestInvitationDurableSMTPPG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	f := newInvitationSMTPFixture(t, true, "accept")
	release := make(chan struct{})
	f.mu.Lock()
	f.release = release
	f.mu.Unlock()
	var releaseOnce sync.Once
	unblock := func() { releaseOnce.Do(func() { close(release) }) }
	defer unblock()
	proof := &pgServiceConfirm{}
	s := invitationService(t, runtime, proof, f.sender(t, false), 101)
	in, id := invitationPrepare(t, s, "session-user-101", "")
	if _, e := s.ConfirmAfterProof(ctx, in.Credential, id); e != access.ErrForbidden {
		t.Fatal("unapproved mail dispatch accepted", e)
	}
	if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, in.Call.OperationID) != 0 {
		t.Fatal("unapproved marker inserted")
	}
	if _, e := s.Confirmation(ctx, "session-user-202", id); e != access.ErrNotFound {
		t.Fatal("cross-user preview exposed", e)
	}
	if _, e := s.ConfirmAfterProof(ctx, "session-user-101-new", id); e != access.ErrForbidden {
		t.Fatal("other same-user Session sent", e)
	}
	proof.mu.Lock()
	proof.approved = true
	proof.mu.Unlock()
	result := make(chan json.RawMessage, 1)
	failure := make(chan error, 1)
	go func() { out, e := s.ConfirmAfterProof(ctx, in.Credential, id); result <- out; failure <- e }()
	deadline := time.Now().Add(5 * time.Second)
	for {
		f.mu.Lock()
		sent := f.deliveries
		f.mu.Unlock()
		if sent == 1 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("first SMTP attempt did not reach DATA")
		}
		time.Sleep(5 * time.Millisecond)
	}
	second := invitationService(t, runtime, proof, f.sender(t, false), 101)
	unknown, e := second.ConfirmAfterProof(ctx, in.Credential, id)
	if e != nil {
		t.Fatal(e)
	}
	invitationResultStatus(t, unknown, "unknown")
	if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_confirmations WHERE id=$1 AND consumed_at IS NULL`, id) != 1 {
		t.Fatal("concurrent request stole first sender's final receipt")
	}
	inspected, e := second.Inspect(ctx, in)
	if e != nil || inspected.State != assistant.AgentToolUnknown {
		t.Fatal("dispatch marker presented as an unapproved draft", e)
	}
	unblock()
	accepted := <-result
	if e := <-failure; e != nil {
		t.Fatal(e)
	}
	invitationResultStatus(t, accepted, "accepted_by_smtp")
	replay, e := second.ConfirmAfterProof(ctx, in.Credential, id)
	if e != nil || !bytes.Equal(replay, accepted) {
		t.Fatal("exact terminal receipt replay changed", e)
	}
	view, e := second.Confirmation(ctx, in.Credential, id)
	if e != nil || view.ConsumedAt == nil || !bytes.Equal(view.Result, accepted) {
		t.Fatal("confirmation readback lost", e)
	}
	completed, e := second.Inspect(ctx, in)
	if e != nil || completed.State != assistant.AgentToolSucceeded {
		t.Fatal("actual accepted receipt not observable", e)
	}
	f.mu.Lock()
	sent := f.deliveries
	f.mu.Unlock()
	if sent != 1 {
		t.Fatal("duplicate SMTP attempt", sent)
	}
	if _, e := runtime.Exec(`UPDATE lmm_assistant.service_confirmations SET result='{}' WHERE id=$1`, id); e == nil {
		t.Fatal("terminal receipt overwritten")
	}
	if _, e := runtime.Exec(`DELETE FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, in.Call.OperationID); e == nil {
		t.Fatal("immutable dispatch marker deleted")
	}
	var pending []byte
	if e := runtime.QueryRow(`SELECT result_bytes FROM lmm_assistant.service_operations WHERE operation_id=$1`, in.Call.OperationID).Scan(&pending); e != nil {
		t.Fatal(e)
	}
	var original assistant.AgentToolResult
	if json.Unmarshal(pending, &original) != nil || original.State != assistant.AgentToolPending {
		t.Fatal("original model operation was rewritten")
	}
}
func TestInvitationCommitAckLossPG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	proof := &pgServiceConfirm{approved: true}
	for _, nth := range []int64{1, 2} {
		t.Run(fmt.Sprint(nth), func(t *testing.T) {
			smtp := newInvitationSMTPFixture(t, true, "accept")
			sender := smtp.sender(t, false)
			s := invitationService(t, runtime, proof, sender, 101)
			label := ""
			if nth == 1 {
				label = "invitation-restart-marker-0001"
			}
			in, id := invitationPrepare(t, s, "session-user-101", label)
			dsn, proxy := invitationCommitProxy(t, nth)
			proxied, e := sql.Open("postgres", dsn)
			if e != nil {
				t.Fatal("cannot open private proxy")
			}
			defer proxied.Close()
			proxied.SetMaxOpenConns(3)
			lossy := invitationService(t, proxied, proof, sender, 101)
			got, e := lossy.ConfirmAfterProof(ctx, in.Credential, id)
			if e != nil {
				t.Fatal("lost ACK did not resolve to durable status", e)
			}
			want := "unknown"
			wantSent := 0
			if nth == 2 {
				want = "accepted_by_smtp"
				wantSent = 1
			}
			invitationResultStatus(t, got, want)
			if proxy.dropped.Load() != 1 {
				t.Fatal("actual COMMIT reply was not dropped", proxy.dropped.Load())
			}
			smtp.mu.Lock()
			sent := smtp.deliveries
			smtp.mu.Unlock()
			if sent != wantSent {
				t.Fatal("SMTP count crossed marker ACK boundary", sent, wantSent)
			}
			if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, in.Call.OperationID) != 1 {
				t.Fatal("real COMMIT not retained")
			}
			replay, e := s.ConfirmAfterProof(ctx, in.Credential, id)
			if e != nil {
				t.Fatal(e)
			}
			invitationResultStatus(t, replay, want)
			if nth == 1 && pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_confirmations WHERE id=$1 AND consumed_at IS NULL`, id) != 1 {
				t.Fatal("unknown marker was falsely consumed")
			}
			smtp.mu.Lock()
			sent = smtp.deliveries
			smtp.mu.Unlock()
			if sent != wantSent {
				t.Fatal("readback replay resent mail")
			}
		})
	}
}
func TestInvitationSMTPAckUnknownPG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	smtp := newInvitationSMTPFixture(t, true, "drop_after_data")
	proof := &pgServiceConfirm{approved: true}
	s := invitationService(t, runtime, proof, smtp.sender(t, false), 101)
	in, id := invitationPrepare(t, s, "session-user-101", "")
	got, e := s.ConfirmAfterProof(ctx, in.Credential, id)
	if e != nil {
		t.Fatal(e)
	}
	invitationResultStatus(t, got, "unknown")
	replay, e := s.ConfirmAfterProof(ctx, in.Credential, id)
	if e != nil || !bytes.Equal(got, replay) {
		t.Fatal("unknown receipt replay changed", e)
	}
	out, e := s.Inspect(ctx, in)
	if e != nil || out.State != assistant.AgentToolUnknown {
		t.Fatal("unknown SMTP receipt presented as success", e)
	}
	smtp.mu.Lock()
	defer smtp.mu.Unlock()
	if smtp.deliveries != 1 {
		t.Fatal("unknown SMTP resent")
	}
}

type invitationAcceptedCounter struct{ count atomic.Int64 }

func (s *invitationAcceptedCounter) SendInvitation(context.Context, string, string, string) InvitationDelivery {
	s.count.Add(1)
	return InvitationDelivery{"accepted_by_smtp", 250}
}
func TestInvitationDurableLimitAndChangedCodePG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	proof := &pgServiceConfirm{approved: true}
	sender := &invitationAcceptedCounter{}
	s := invitationService(t, runtime, proof, sender, 202)
	// This case isolates durable rate semantics; TLS wire behavior is exercised
	// by the other actual PG cases. No external recipient is contacted.
	for n := 0; n < 120; n++ {
		in, id := invitationPrepare(t, s, "session-user-202", "")
		got, e := s.ConfirmAfterProof(ctx, in.Credential, id)
		if e != nil {
			t.Fatal("authorized invitation below limit rejected", n, e)
		}
		invitationResultStatus(t, got, "accepted_by_smtp")
	}
	last, id := invitationPrepare(t, s, "session-user-202", "")
	if _, e := s.ConfirmAfterProof(ctx, last.Credential, id); e != access.ErrLimit {
		t.Fatal("121st durable dispatch not refused", e)
	}
	if sender.count.Load() != 120 || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, last.Call.OperationID) != 0 {
		t.Fatal("over-limit attempt sent or claimed")
	}
	own := invitationService(t, runtime, proof, sender, 101)
	in, id := invitationPrepare(t, own, "session-user-101", "")
	reader := own.dependencies.Invitations.(*invitationTestReader)
	changed := "Ab09"
	reader.facts.InvitationCode = &changed
	if _, e := own.ConfirmAfterProof(ctx, in.Credential, id); e != access.ErrConflict {
		t.Fatal("changed case-sensitive own code dispatched", e)
	}
	if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, in.Call.OperationID) != 0 {
		t.Fatal("changed source claimed dispatch")
	}
}
func TestInvitationRestartMarkerNeverResends(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_network_dispatch WHERE operation_id='invitation-restart-marker-0001'`) != 1 {
		t.Fatal("actual pre-restart committed marker absent")
	}
	sender := &invitationAcceptedCounter{}
	proof := &pgServiceConfirm{approved: true}
	s := invitationService(t, runtime, proof, sender, 101)
	id := digest([]byte("assistant-confirmation:invitation-restart-marker-0001"))
	raw, e := s.ConfirmAfterProof(ctx, "session-user-101", id)
	if e != nil {
		t.Fatal(e)
	}
	invitationResultStatus(t, raw, "unknown")
	if sender.count.Load() != 0 || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_confirmations WHERE id=$1 AND consumed_at IS NULL`, id) != 1 {
		t.Fatal("restart resent or stole unknown outcome")
	}
}
