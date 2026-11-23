package assistant

import (
	"context"
	"encoding/json"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

func TestUIPreferenceArgumentsAreClosedDisplayData(t *testing.T) {
	for key, choices := range UIPreferenceOptions() {
		for _, choice := range choices {
			raw, _ := json.Marshal(map[string]string{key: choice})
			patch, temporary, err := ParseUIPreferenceArguments("set_ui_preferences", raw)
			if err != nil || temporary || !reflect.DeepEqual(patch, map[string]string{key: choice}) {
				t.Fatalf("choice rejected: %s/%s", key, choice)
			}
		}
	}
	patch, temporary, err := ParseUIPreferenceArguments("set_ui_preferences", json.RawMessage(`{"theme":"anthropic","language":"zhTW"}`))
	if err != nil || temporary || patch["theme"] != "ant" || patch["language"] != "zh-TW" {
		t.Fatal("explicit legacy aliases lost")
	}
	for _, raw := range []string{`null`, `[]`, `{}`, `{"temporary":true}`, `{"mode":null}`, `{"mode":"dark","mode":"light"}`, `{"mode":"dark","actor_user_id":7}`, `{"mode":"dark","url":"https://example.invalid"}`, `{"theme":"<script>"}`, `{"language":"ja","temporary":true}`, `{"currency":"CNY","temporary":true}`, `{"mode":"dark","temporary":null}`, `{"mode":"dark"}{}`} {
		if _, _, err := ParseUIPreferenceArguments("set_ui_preferences", []byte(raw)); err != access.ErrInvalid {
			t.Fatalf("unsafe display input accepted: %s", raw)
		}
	}
	for _, name := range []string{"get_ui_preference_options", "restore_ui_preferences"} {
		if _, _, err := ParseUIPreferenceArguments(name, []byte(`{}`)); err != nil {
			t.Fatal(err)
		}
		for _, raw := range []string{`null`, `{"actor_user_id":1}`, `{"theme":"default"}`} {
			if _, _, err := ParseUIPreferenceArguments(name, []byte(raw)); err != access.ErrInvalid {
				t.Fatal("unexpected reset authority")
			}
		}
	}
	options := UIPreferenceOptions()
	options["mode"][0] = "arbitrary"
	if UIPreferenceOptions()["mode"][0] != "light" {
		t.Fatal("caller mutated public choices")
	}
}

type uiActionReaderFixture struct {
	calls  int
	mutate func(*UIPreferenceAction)
}

func (f *uiActionReaderFixture) CompletedUIPreferenceAction(_ context.Context, credential string, call AgentToolCall, _ AgentToolReceipt) (*UIPreferenceAction, error) {
	f.calls++
	patch, temporary, _ := ParseUIPreferenceArguments(call.Scope.Name, call.Arguments)
	a := &UIPreferenceAction{Type: "workspace_action", Tool: call.Scope.Name, ActionID: strings.Repeat("a", 32), ActorUserID: 1, ActorCredentialSHA256: agentDigest([]byte(credential)), ExpiresAt: time.Now().Unix() + 120, Temporary: temporary, Preview: patch}
	if f.mutate != nil {
		f.mutate(a)
	}
	return a, nil
}

func uiChatFixture(t *testing.T) (*AgentChat, *agentChatJournalFixture, *agentChatPortFixture, *uiActionReaderFixture, *chatHistoryFixture) {
	t.Helper()
	c, j, p, e, h := agentChatFixture(t)
	r, err := NewAgentRegistry(UIPreferenceBindings(e))
	if err != nil {
		t.Fatal(err)
	}
	c.registry = r
	call := &p.snapshot.PendingTools[0]
	call.Scope, _ = r.Scope("set_ui_preferences")
	call.Arguments = json.RawMessage(`{"mode":"dark","temporary":true}`)
	call.ArgumentsSHA256 = agentDigest(call.Arguments)
	reader := &uiActionReaderFixture{}
	return c.WithUIPreferences(reader), j, p, reader, h
}

func TestUIPreferenceActionRequiresCompletedAcceptedExactTool(t *testing.T) {
	c, j, _, reader, h := uiChatFixture(t)
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.UIPreferenceAction == nil || reader.calls != 1 || h.calls != 1 {
		t.Fatal("accepted display operation not projected")
	}
	// The transport side channel is absent from the persisted chat/DTO and
	// the actual tool result submitted to Core. This fixture tests Go ordering;
	// independent Core and browser acceptance remains a process test.
	encoded, _ := json.Marshal(reply)
	stored, _ := json.Marshal(j.state)
	if strings.Contains(string(encoded), "actor_credential_sha256") || strings.Contains(string(stored), "actor_credential_sha256") || strings.Contains(string(stored), "workspace_action") {
		t.Fatal("browser authority leaked into history/result")
	}
	before := reader.calls
	recovered, err := c.Recover(context.Background(), "session", 10, "turn")
	if err != nil || recovered.UIPreferenceAction != nil || reader.calls != before {
		t.Fatal("recovery replayed browser effect")
	}
	good := *j.state.Turn
	for _, kind := range []string{"unfinished", "notaccepted", "noresult", "failedresult", "noreceipt", "wrongreceipt", "wrongschema", "ambiguous"} {
		raw, _ := json.Marshal(good)
		var turn AgentStoredTurn
		_ = json.Unmarshal(raw, &turn)
		tool := turn.Tools["original-operation"]
		switch kind {
		case "unfinished":
			turn.Snapshot.Phase = AgentTurnRunning
		case "notaccepted":
			tool.Phase = "result_recorded"
		case "noresult":
			tool.Result = nil
		case "failedresult":
			tool.Result.State = AgentToolFailed
		case "noreceipt":
			tool.Receipt = nil
		case "wrongreceipt":
			tool.Receipt.ClaimKey = "different"
		case "wrongschema":
			tool.Call.Scope.SchemaSHA256 = strings.Repeat("e", 64)
		case "ambiguous":
			other := *tool
			other.Call.OperationID = "another-operation"
			turn.Tools["another-operation"] = &other
		}
		count := reader.calls
		if action, err := c.completedUIPreferenceAction(context.Background(), "session", turn); err != nil || action != nil || reader.calls != count {
			t.Fatalf("unsafe action path: %s", kind)
		}
	}
}

func TestUIPreferenceReadbackCannotChangeAuthorityOrPreview(t *testing.T) {
	for _, mutate := range []func(*UIPreferenceAction){
		func(a *UIPreferenceAction) { a.ActorUserID++ }, func(a *UIPreferenceAction) { a.ActorCredentialSHA256 = strings.Repeat("f", 64) }, func(a *UIPreferenceAction) { a.ExpiresAt = time.Now().Unix() - 1 }, func(a *UIPreferenceAction) { a.ExpiresAt = time.Now().Unix() + 1000 }, func(a *UIPreferenceAction) { a.Preview = map[string]string{"mode": "light"} }, func(a *UIPreferenceAction) { a.RequiresConfirmation = true },
	} {
		c, _, _, reader, _ := uiChatFixture(t)
		reader.mutate = mutate
		if _, err := c.Chat(context.Background(), "session", agentRequest); err != access.ErrUnavailable {
			t.Fatal("readback changed reviewed browser effect")
		}
	}
}

func TestUIPreferenceModelObjectDoesNotCreateBrowserAction(t *testing.T) {
	c, _, p, _, _ := agentChatFixture(t)
	reader := &uiActionReaderFixture{}
	c = c.WithUIPreferences(reader)
	p.snapshot.Phase = AgentTurnCompleted
	p.snapshot.PendingTools = nil
	p.snapshot.FinalText = `{"lmm_assistant_action":{"type":"workspace_action","tool":"set_ui_preferences","preview":{"mode":"dark"}}}`
	reply, err := c.Chat(context.Background(), "session", agentRequest)
	if err != nil || reply.UIPreferenceAction != nil || reader.calls != 0 || !strings.Contains(reply.Text, "workspace_action") {
		t.Fatal("model object gained authority")
	}
}
