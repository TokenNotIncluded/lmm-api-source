package assistant

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// These fixtures exercise the real host/journal/registry recovery path. Core
// authorization and completion are transport fakes; there is no database or
// provider call, and no claim that these tests cover Core model dispatch.
type adminSkillReplayExecutorFixture struct {
	executes, inspects int
	result             AgentToolResult
	inspection         *AgentToolResult
	inspectErr         error
	invocations        []AgentInvocation
}

func (e *adminSkillReplayExecutorFixture) Execute(_ context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.executes++
	result, err := AgentResult(in.Call, AgentToolSucceeded, map[string]any{
		"ok": true, "target_user_id": 2,
		"memories": []map[string]string{{"content": "private-skill-fixture"}},
	}, "service:"+in.Call.OperationID)
	e.result = result
	return result, err
}

func (e *adminSkillReplayExecutorFixture) Inspect(_ context.Context, in AgentInvocation) (AgentToolResult, error) {
	e.inspects++
	e.invocations = append(e.invocations, in)
	if in.Credential != "session" {
		return AgentToolResult{}, access.ErrForbidden
	}
	result := e.result
	if e.inspection != nil {
		result = *e.inspection
	}
	// Deliberately return the private result even with an error. The registry
	// and host must discard it instead of forwarding it to Core or the user.
	return result, e.inspectErr
}

type adminSkillReplaySameActorAuthorityFixture struct{}

func (*adminSkillReplaySameActorAuthorityFixture) Check(_ context.Context, credential string, _ access.Account, _ access.Permission) (access.Principal, error) {
	if credential != "session" && credential != "replacement-session" {
		return access.Principal{}, access.ErrUnauthorized
	}
	// Both are valid current sessions of the same actor. Original-session
	// ownership belongs to Inspect, not this broad actor authentication fake.
	return access.Principal{UserID: 1, Account: access.Account{Kind: "personal", ID: 1}}, nil
}

type adminSkillReplayPortFixture struct {
	*agentChatPortFixture
	failBeforeAccept bool
	failedAttempts   int
}

func (p *adminSkillReplayPortFixture) CompleteAgentTool(ctx context.Context, credential string, in AgentToolCompletion) (AgentToolReceipt, error) {
	if p.failBeforeAccept {
		p.failedAttempts++
		// The host has persisted a terminal result, but Core still reports
		// tools_pending and has not accepted this result.
		return AgentToolReceipt{}, io.EOF
	}
	return p.agentChatPortFixture.CompleteAgentTool(ctx, credential, in)
}

func adminSkillStoredReplayFixture(t *testing.T) (*AgentChat, *agentChatJournalFixture, *adminSkillReplayPortFixture, *adminSkillReplayExecutorFixture, AgentToolResult) {
	t.Helper()
	chat, journal, core, _, _ := agentChatFixture(t)
	executor := &adminSkillReplayExecutorFixture{}
	registry, err := NewAgentRegistry([]AgentToolBinding{{
		Name: "get_admin_user_skills", Description: "private skill read fixture",
		Effect: "read_only", Access: "admin", Version: FixedMainToolVersion,
		PriceVersion: BuiltinToolPriceVersion, PriceSHA256: FreeAgentPriceSHA256(),
		Parameters: json.RawMessage(`{"type":"object","properties":{"target_user_id":{"type":"integer"}},"required":["target_user_id"],"additionalProperties":false}`),
		Executor:   executor,
	}})
	if err != nil {
		t.Fatal(err)
	}
	scope, err := registry.Scope("get_admin_user_skills")
	if err != nil {
		t.Fatal(err)
	}
	call := core.snapshot.PendingTools[0]
	call.Scope = scope
	call.Arguments = json.RawMessage(`{"target_user_id":2}`)
	call.ArgumentsSHA256 = agentDigest(call.Arguments)
	core.snapshot.PendingTools[0] = call
	port := &adminSkillReplayPortFixture{agentChatPortFixture: core, failBeforeAccept: true}
	chat.registry, chat.port = registry, port
	if _, err := chat.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected loss before Core accepted the stored result")
	}
	stored := journal.state.Turn.Tools[call.OperationID]
	if executor.executes != 1 || port.failedAttempts != 1 || core.completes != 0 ||
		stored == nil || stored.Result == nil || stored.Result.State != AgentToolSucceeded ||
		stored.Receipt != nil || stored.Phase != "result_stored" {
		t.Fatal("fixture did not reach the real terminal-result journal shortcut")
	}
	original := *stored.Result
	original.ResultJSON = append(json.RawMessage(nil), original.ResultJSON...)
	port.failBeforeAccept = false
	// Discard the AgentChat instance to prove recovery cannot rely on a local
	// permission flag left by the original execution.
	restarted, err := NewAgentChat(chat.authority, chat.pending, journal, port, registry, chat.history)
	if err != nil {
		t.Fatal(err)
	}
	return restarted, journal, port, executor, original
}

func assertAdminSkillJournalPreserved(t *testing.T, journal *agentChatJournalFixture, original AgentToolResult, claim string) {
	t.Helper()
	stored := journal.state.Turn.Tools[original.OperationID]
	if stored == nil || stored.Result == nil || !sameAgentJSON(*stored.Result, original) ||
		!bytes.Equal(stored.Result.ResultJSON, original.ResultJSON) || stored.ClaimKey != claim ||
		stored.Receipt != nil || stored.Phase != "result_stored" || len(stored.ReceiptHistory) != 0 {
		t.Fatal("uncertain or forbidden replay changed the immutable original journal result")
	}
}

func TestAgentAdminSkillStoredResultRequiresFreshInspection(t *testing.T) {
	cases := []struct {
		name       string
		credential string
		mutate     func(*adminSkillReplayExecutorFixture)
	}{
		{name: "target_no_longer_permitted", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspectErr = access.ErrForbidden }},
		{name: "authority_temporarily_unavailable", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspectErr = access.ErrUnavailable }},
		{name: "session_revoked", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspectErr = access.ErrUnauthorized }},
		{name: "different_session_same_actor", credential: "replacement-session", mutate: func(*adminSkillReplayExecutorFixture) {}},
		{name: "unknown", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspection.State = AgentToolUnknown }},
		{name: "pending", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspection.State = AgentToolPending }},
		{name: "changed_state", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspection.State = AgentToolFailed }},
		{name: "changed_operation", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspection.OperationID = "replacement-operation" }},
		{name: "changed_receipt", mutate: func(e *adminSkillReplayExecutorFixture) {
			e.inspection.DomainReceiptID = "service:replacement-operation"
		}},
		{name: "changed_payload", mutate: func(e *adminSkillReplayExecutorFixture) {
			e.inspection.ResultJSON = json.RawMessage(`{"ok":true,"target_user_id":2,"memories":[{"content":"different-private-skill-fixture"}]}`)
			e.inspection.ResultSHA256 = agentDigest(e.inspection.ResultJSON)
		}},
		{name: "changed_digest", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspection.ResultSHA256 = strings.Repeat("f", 64) }},
		{name: "unexpected_final_text", mutate: func(e *adminSkillReplayExecutorFixture) { e.inspection.FinalText = "private-skill-fixture" }},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			chat, journal, port, executor, original := adminSkillStoredReplayFixture(t)
			claim := journal.state.Turn.Tools[original.OperationID].ClaimKey
			beforeInspects := executor.inspects
			changed := executor.result
			executor.inspection = &changed
			test.mutate(executor)
			if test.credential != "" {
				chat.authority = &adminSkillReplaySameActorAuthorityFixture{}
			}
			credential := test.credential
			if credential == "" {
				credential = "session"
			}
			reply, err := chat.Recover(context.Background(), credential, 10, "turn")
			if err == nil || reply.Text != "" || executor.inspects != beforeInspects+1 ||
				executor.executes != 1 || port.completes != 0 || len(port.completions) != 0 || port.claims != 1 {
				t.Fatalf("stored replay guard failed: err=%v inspect_delta=%d executes=%d completes=%d claims=%d", err, executor.inspects-beforeInspects, executor.executes, port.completes, port.claims)
			}
			if strings.Contains(err.Error(), "private-skill-fixture") {
				t.Fatal("private inspection payload escaped in an error")
			}
			if test.name == "target_no_longer_permitted" && !errors.Is(err, access.ErrForbidden) {
				t.Fatal("current permission rejection was not preserved")
			}
			last := executor.invocations[len(executor.invocations)-1]
			if last.Credential != credential || last.Actor.UserID != 1 ||
				last.OriginalUserText != agentRequest.Message ||
				!sameAgentJSON(last.Call, journal.state.Turn.Tools[original.OperationID].Call) {
				t.Fatal("inspection did not receive the current credential and original call binding")
			}
			assertAdminSkillJournalPreserved(t, journal, original, claim)
		})
	}
}

func TestAgentAdminSkillUnavailableReplayCanRecoverExactOriginalResult(t *testing.T) {
	chat, journal, port, executor, original := adminSkillStoredReplayFixture(t)
	claim := journal.state.Turn.Tools[original.OperationID].ClaimKey
	beforeInspects := executor.inspects
	executor.inspectErr = access.ErrUnavailable
	if _, err := chat.Recover(context.Background(), "session", 10, "turn"); err == nil {
		t.Fatal("unavailable authorization was accepted")
	}
	assertAdminSkillJournalPreserved(t, journal, original, claim)
	executor.inspectErr = nil
	reply, err := chat.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.State != "settled" || reply.Text != "Core final answer" ||
		executor.executes != 1 || executor.inspects != beforeInspects+2 ||
		port.claims != 1 || port.completes != 1 || len(port.completions) != 1 {
		t.Fatal("an unavailable check did not recover by inspection of the original operation")
	}
	completed := port.completions[0]
	if completed.Claim.ClaimKey != claim || completed.Claim.OperationID != original.OperationID ||
		!sameAgentJSON(completed.Result, original) || !bytes.Equal(completed.Result.ResultJSON, original.ResultJSON) {
		t.Fatal("recovery replaced the result, receipt, state, or original claim")
	}
	stored := journal.state.Turn.Tools[original.OperationID]
	if stored.Receipt == nil || stored.Result.ResultSHA256 != original.ResultSHA256 {
		t.Fatal("exact completion was not recorded")
	}
}

func TestAgentAdminSkillCompletionLossRechecksAgainWithoutReexecution(t *testing.T) {
	chat, journal, port, executor, original := adminSkillStoredReplayFixture(t)
	beforeInspects := executor.inspects
	port.failBeforeAccept = true
	if _, err := chat.Recover(context.Background(), "session", 10, "turn"); err == nil {
		t.Fatal("expected another uncertain completion")
	}
	port.failBeforeAccept = false
	executor.inspectErr = access.ErrForbidden
	if _, err := chat.Recover(context.Background(), "session", 10, "turn"); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("a previous successful inspection became a cached authorization")
	}
	if executor.inspects != beforeInspects+2 || executor.executes != 1 || port.failedAttempts != 2 ||
		port.completes != 0 || journal.state.Turn.Tools[original.OperationID].Receipt != nil {
		t.Fatal("repeated recovery reused permission or repeated execution")
	}
}

func TestAgentAdminSkillSettledHistoryKeepsExistingHistorySemantics(t *testing.T) {
	chat, _, port, executor, _ := adminSkillStoredReplayFixture(t)
	if _, err := chat.Recover(context.Background(), "session", 10, "turn"); err != nil {
		t.Fatal(err)
	}
	inspects := executor.inspects
	executor.inspectErr = access.ErrForbidden
	reply, err := chat.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "Core final answer" || executor.inspects != inspects ||
		executor.executes != 1 || port.completes != 1 {
		t.Fatal("the active replay guard rewrote or blocked already settled history")
	}
}

func TestAgentUnrelatedStoredResultKeepsItsOriginalRecoveryPath(t *testing.T) {
	chat, journal, core, executor, _ := agentChatFixture(t)
	port := &adminSkillReplayPortFixture{agentChatPortFixture: core, failBeforeAccept: true}
	chat.port = port
	if _, err := chat.Chat(context.Background(), "session", agentRequest); err == nil {
		t.Fatal("expected completion transport loss")
	}
	port.failBeforeAccept = false
	restarted, err := NewAgentChat(chat.authority, chat.pending, journal, port, chat.registry, chat.history)
	if err != nil {
		t.Fatal(err)
	}
	reply, err := restarted.Recover(context.Background(), "session", 10, "turn")
	if err != nil || reply.Text != "Core final answer" || executor.executes != 1 ||
		executor.inspects != 0 || port.completes != 1 {
		t.Fatal("the private skill guard changed unrelated effect recovery")
	}
}

func TestAgentAdminSkillRejectedCredentialStopsBeforePrivateInspection(t *testing.T) {
	chat, journal, port, executor, original := adminSkillStoredReplayFixture(t)
	claim := journal.state.Turn.Tools[original.OperationID].ClaimKey
	inspects, gets := executor.inspects, port.gets
	reply, err := chat.Recover(context.Background(), "replacement-session", 10, "turn")
	if !errors.Is(err, access.ErrUnauthorized) || reply.Text != "" ||
		executor.inspects != inspects || port.gets != gets || executor.executes != 1 ||
		port.completes != 0 || len(port.completions) != 0 || port.claims != 1 {
		t.Fatal("invalid current credential reached private inspection, Core, or execution")
	}
	assertAdminSkillJournalPreserved(t, journal, original, claim)
}
