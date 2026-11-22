package assistant

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"strings"
	"testing"
	"time"
)

type invitationPendingExecutor struct {
	result   *AgentToolResult
	executes int
}

func (e *invitationPendingExecutor) Execute(_ context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.executes++
	r, err := AgentResult(in.Call, AgentToolPending, map[string]any{"status": "awaiting_confirmation"}, "confirmation-reference")
	e.result = &r
	return r, err
}
func (e *invitationPendingExecutor) Inspect(_ context.Context, in AgentInvocation) (AgentToolResult, error) {
	if e.result != nil {
		return *e.result, nil
	}
	return AgentToolResult{}, access.ErrNotFound
}

type invitationActionReader struct {
	calls  int
	mutate func(*InvitationConfirmationAction)
}

func (r *invitationActionReader) PendingInvitationConfirmation(_ context.Context, credential string, c AgentToolCall, _ AgentToolResult) (*InvitationConfirmationAction, error) {
	r.calls++
	a := &InvitationConfirmationAction{Type: "invitation_confirmation", Tool: "send_invitation", ConfirmationID: strings.Repeat("a", 64), OperationID: c.OperationID, ActorUserID: 1, ActorCredentialSHA256: agentDigest([]byte(credential)), ExpiresAt: time.Now().Unix() + 120, Recipient: "friend@example.invalid"}
	if r.mutate != nil {
		r.mutate(a)
	}
	return a, nil
}
func invitationChatFixture(t *testing.T) (*AgentChat, *agentChatJournalFixture, *agentChatPortFixture, *invitationPendingExecutor, *invitationActionReader, *chatHistoryFixture) {
	t.Helper()
	c, j, p, _, h := agentChatFixture(t)
	e := &invitationPendingExecutor{}
	r, err := NewAgentRegistry([]AgentToolBinding{{Name: "send_invitation", Description: "fixture", Effect: "confirmation", Access: "user", Version: FixedMainToolVersion, PriceVersion: BuiltinToolPriceVersion, PriceSHA256: FreeAgentPriceSHA256(), Parameters: json.RawMessage(`{"type":"object","properties":{"email":{"type":"string"}},"required":["email"],"additionalProperties":false}`), Executor: e}})
	if err != nil {
		t.Fatal(err)
	}
	c.registry = r
	call := &p.snapshot.PendingTools[0]
	call.Scope, _ = r.Scope("send_invitation")
	call.Arguments = json.RawMessage(`{"email":"friend@example.invalid"}`)
	call.ArgumentsSHA256 = agentDigest(call.Arguments)
	reader := &invitationActionReader{}
	return c.WithInvitationConfirmations(reader), j, p, e, reader, h
}
func TestInvitationPendingBrowserActionRequiresActualPendingOperation(t *testing.T) {
	c, j, p, e, r, h := invitationChatFixture(t)
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.State != "tools_pending" || reply.InvitationConfirmation == nil || r.calls != 1 || e.executes != 1 || p.completes != 0 || h.calls != 0 {
		t.Fatal(err, r.calls, e.executes, p.completes, h.calls)
	}
	raw, _ := json.Marshal(reply)
	stored, _ := json.Marshal(j.state)
	if strings.Contains(string(raw), "actor_credential_sha256") || strings.Contains(string(stored), "actor_credential_sha256") {
		t.Fatal("private browser binding leaked into model/history")
	}
	recovered, err := c.Recover(context.Background(), "session", 10, "turn")
	if err != nil || recovered.InvitationConfirmation != nil || r.calls != 1 || e.executes != 1 {
		t.Fatal("recovery produced browser effect", err)
	}
	// A fresh explicit resume may show the same unconsumed original challenge.
	reply, err = c.Resume(context.Background(), "session", 10, "turn")
	if err != nil || reply.InvitationConfirmation == nil || e.executes != 1 || r.calls != 2 {
		t.Fatal("resume duplicated effect or lost original challenge", err)
	}
}
func TestInvitationBrowserReadbackRejectsChangedAuthority(t *testing.T) {
	for _, mutate := range []func(*InvitationConfirmationAction){
		func(a *InvitationConfirmationAction) { a.ActorUserID++ },
		func(a *InvitationConfirmationAction) { a.ActorCredentialSHA256 = strings.Repeat("f", 64) },
		func(a *InvitationConfirmationAction) { a.OperationID = "other" },
		func(a *InvitationConfirmationAction) { a.ExpiresAt = time.Now().Unix() - 1 },
		func(a *InvitationConfirmationAction) { a.ExpiresAt = time.Now().Unix() + 1000 },
		func(a *InvitationConfirmationAction) { a.Tool = "arbitrary" },
	} {
		c, _, _, _, r, _ := invitationChatFixture(t)
		r.mutate = mutate
		if _, err := c.Chat(context.Background(), "session", agentRequest); err != access.ErrUnavailable {
			t.Fatal("altered confirmation projected", err)
		}
	}
}
func TestInvitationModelJSONAndAmbiguousToolsCannotCreateBrowserAction(t *testing.T) {
	c, j, p, _, r, _ := invitationChatFixture(t)
	p.snapshot.Phase = AgentTurnCompleted
	p.snapshot.PendingTools = nil
	p.snapshot.FinalText = `{"lmm_assistant_invitation":{"type":"invitation_confirmation"}}`
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.InvitationConfirmation != nil || r.calls != 0 {
		t.Fatal("model text became confirmation", err)
	}
	c, j, p, _, r, _ = invitationChatFixture(t)
	if _, err = c.Chat(context.Background(), "session", agentRequest); err != nil {
		t.Fatal(err)
	}
	duplicate := *j.state.Turn.Tools["original-operation"]
	duplicate.Call.OperationID = "other-operation"
	j.state.Turn.Tools["other-operation"] = &duplicate
	j.state.Turn.Snapshot.PendingTools = append(j.state.Turn.Snapshot.PendingTools, duplicate.Call)
	actor, _ := c.actor(context.Background(), "session")
	action, _ := c.pendingInvitationConfirmation(context.Background(), "session", actor, 10, "turn")
	if action != nil || r.calls != 1 {
		t.Fatal("ambiguous pending actions projected")
	}
}
