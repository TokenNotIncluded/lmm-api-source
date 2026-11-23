package assistant_services

import (
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"testing"
)

func TestInvitationPendingAuthorityReadbackPG(t *testing.T) {
	ctx := context.Background()
	db := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	proof := &pgServiceConfirm{}
	sender := &invitationAcceptedCounter{}
	service := invitationService(t, db, proof, sender, 101)
	in := pgServiceInvocation("session-user-101", "send_invitation", `{"email":"visitor+tag@example.invalid"}`)
	registry, registryErr := assistant.NewAgentRegistry(service.AvailableBindings())
	if registryErr != nil {
		t.Fatal(registryErr)
	}
	scope, scopeErr := registry.Scope("send_invitation")
	if scopeErr != nil {
		t.Fatal(scopeErr)
	}
	in.Call.Scope = scope
	in.Call.OperationID = "invitation-pending-readback-0001"
	prepared := pgServiceMust(t, service, in)
	id := pgServiceConfirmationID(t, prepared)
	pending, e := service.Inspect(ctx, in)
	if e != nil || pending.State != assistant.AgentToolPending {
		t.Fatal(e)
	}
	action, e := service.PendingInvitationConfirmation(ctx, in.Credential, in.Call, pending)
	if e != nil || action == nil || action.ConfirmationID != id || action.ActorUserID != 101 || action.Recipient != "visitor+tag@example.invalid" || action.ActorCredentialSHA256 != digest([]byte(in.Credential)) {
		t.Fatal("actual row binding missing", e)
	}
	view, e := service.Confirmation(ctx, in.Credential, id)
	if e != nil || view.HumanConfirmed || view.DispatchStarted || sender.count.Load() != 0 {
		t.Fatal("unapproved preview dispatched or forged proof", e)
	}
	if a, e := service.PendingInvitationConfirmation(ctx, "session-user-101-new", in.Call, pending); a != nil || (e != nil && e != access.ErrForbidden && e != access.ErrConflict) {
		t.Fatal("same-user different Session got action", e)
	}
	changed := pending
	changed.ResultSHA256 = digest([]byte("other result"))
	if _, e := service.PendingInvitationConfirmation(ctx, in.Credential, in.Call, changed); e != access.ErrConflict {
		t.Fatal("changed result accepted", e)
	}
	proof.mu.Lock()
	proof.approved = true
	proof.mu.Unlock()
	view, e = service.Confirmation(ctx, in.Credential, id)
	if e != nil || !view.HumanConfirmed || view.DispatchStarted || sender.count.Load() != 0 {
		t.Fatal("actual proof not observed read-only", e)
	}
	result, e := service.ConfirmAfterProof(ctx, in.Credential, id)
	if e != nil {
		t.Fatal(e)
	}
	invitationResultStatus(t, result, "accepted_by_smtp")
	view, e = service.Confirmation(ctx, in.Credential, id)
	if e != nil || view.HumanConfirmed || !view.DispatchStarted || view.ConsumedAt == nil || sender.count.Load() != 1 {
		t.Fatal("terminal status not preserved", e)
	}
	if action, e := service.PendingInvitationConfirmation(ctx, in.Credential, in.Call, pending); e != nil || action != nil {
		t.Fatal("completed operation exposed fresh send action", e)
	}
}
