package assistant_services

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// PostgreSQL persistence is real; Core accepted receipts and identity checks
// below are explicitly synthetic. The full Core+Go process suite separately
// verifies that only the Core-accepted side channel reaches the browser.
func TestUIPreferenceDurableReadbackBindsOriginalSessionAndExpiry(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	s, err := New(ctx, runtime, pgServiceAuthority{}, Dependencies{})
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now().Truncate(time.Second)
	s.now = func() time.Time { return now }
	registry, err := assistant.NewAgentRegistry(assistant.UIPreferenceBindings(s))
	if err != nil {
		t.Fatal(err)
	}
	in := pgServiceInvocation("session-user-101", "set_ui_preferences", `{"theme":"ant","mode":"dark","temporary":true}`)
	in.Call.Scope, err = registry.Scope(in.Call.Scope.Name)
	if err != nil {
		t.Fatal(err)
	}
	first := pgServiceMust(t, s, in)
	receipt := assistant.AgentToolReceipt{OperationID: in.Call.OperationID, State: assistant.AgentToolSucceeded, ResultSHA256: first.ResultSHA256, DomainReceiptID: first.DomainReceiptID, ClaimKey: "synthetic-core-claim"}
	for _, forbidden := range []string{"workspace_action", "actor_credential_sha256", in.Credential, digest([]byte(in.Credential))} {
		if strings.Contains(string(first.ResultJSON), forbidden) {
			t.Fatal("browser-only authority entered tool result")
		}
	}
	a, err := s.CompletedUIPreferenceAction(ctx, in.Credential, in.Call, receipt)
	if err != nil || a == nil || a.ActorUserID != 101 || a.ActorCredentialSHA256 != digest([]byte(in.Credential)) || a.ExpiresAt != now.Unix()+120 || !a.Temporary || a.Preview["theme"] != "ant" {
		t.Fatal("original accepted operation failed exact projection")
	}
	if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_operations WHERE operation_id=$1`, in.Call.OperationID) != 1 {
		t.Fatal("missing durable operation")
	}
	for _, credential := range []string{"session-user-101-new", "session-user-202"} {
		if action, err := s.CompletedUIPreferenceAction(ctx, credential, in.Call, receipt); err != nil || action != nil {
			t.Fatal("action rebound to refreshed/other credential")
		}
	}
	changed := in.Call
	changed.Arguments = json.RawMessage(`{"mode":"light"}`)
	changed.ArgumentsSHA256 = digest(changed.Arguments)
	if action, err := s.CompletedUIPreferenceAction(ctx, in.Credential, changed, receipt); err != nil || action != nil {
		t.Fatal("altered call reused stored authority")
	}
	wrong := receipt
	wrong.ResultSHA256 = strings.Repeat("e", 64)
	if _, err := s.CompletedUIPreferenceAction(ctx, in.Credential, in.Call, wrong); err != access.ErrConflict {
		t.Fatal("mismatched Core result accepted")
	}
	badScope := in.Call
	badScope.Scope.SchemaSHA256 = strings.Repeat("f", 64)
	if _, err := s.CompletedUIPreferenceAction(ctx, in.Credential, badScope, receipt); err != access.ErrConflict {
		t.Fatal("unreviewed scope accepted")
	}
	// Reopening the actual runtime preserves the original deadline, never
	// retries a display mutation or manufactures a new action ID.
	reopened, err := New(ctx, pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE"), pgServiceAuthority{}, Dependencies{})
	if err != nil {
		t.Fatal(err)
	}
	reopened.now = func() time.Time { return now.Add(30 * time.Second) }
	second := pgServiceMust(t, reopened, in)
	if string(second.ResultJSON) != string(first.ResultJSON) || second.ResultSHA256 != first.ResultSHA256 {
		t.Fatal("replay renewed action")
	}
	b, err := reopened.CompletedUIPreferenceAction(ctx, in.Credential, in.Call, receipt)
	if err != nil || b == nil || b.ActionID != a.ActionID || b.ExpiresAt != a.ExpiresAt {
		t.Fatal("restart changed original action binding")
	}
	reopened.now = func() time.Time { return now.Add(120 * time.Second) }
	if a, err := reopened.CompletedUIPreferenceAction(ctx, in.Credential, in.Call, receipt); err != nil || a != nil {
		t.Fatal("expired operation became active")
	}
}

func TestUIPreferenceOptionsAndCostHaveNoAccountEffects(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	s, err := New(ctx, runtime, pgServiceAuthority{}, Dependencies{})
	if err != nil {
		t.Fatal(err)
	}
	tables := []string{"memories", "profiles", "service_network_dispatch", "service_confirmations", "site_issues", "site_issue_events", "platform_support", "platform_support_messages", "platform_support_todos"}
	before := map[string]int{}
	for _, name := range tables {
		before[name] = pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.`+name)
	}
	for _, entry := range []struct{ name, args string }{{"get_ui_preference_options", `{}`}, {"calculate_cost", `{"input_tokens":1000000,"output_tokens":0,"input_usd_per_million":2,"output_usd_per_million":3}`}} {
		out := pgServiceMust(t, s, pgServiceInvocation("session-user-101", entry.name, entry.args))
		var result map[string]any
		if json.Unmarshal(out.ResultJSON, &result) != nil || result["ok"] != true {
			t.Fatal("missing query result")
		}
		if entry.name == "get_ui_preference_options" && (result["currency_scope"] != "display_only" || result["current_preferences_available"] != false) {
			t.Fatal("choices claimed browser state or money authority")
		}
		if entry.name == "calculate_cost" && (result["total_cost_usd"] != float64(2) || result["estimate_only"] != true) {
			t.Fatal("estimate result mismatch")
		}
	}
	for _, name := range tables {
		if before[name] != pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.`+name) {
			t.Fatalf("read changed business table %s", name)
		}
	}
}
