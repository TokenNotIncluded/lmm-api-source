package assistant_services

import "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"

type uiPreparation struct {
	OK        bool   `json:"ok"`
	Status    string `json:"status"`
	Applied   bool   `json:"applied"`
	ExpiresAt int64  `json:"expires_at"`
}

func (s *Service) uiPreferenceTool(in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	_, _, err := assistant.ParseUIPreferenceArguments(in.Call.Scope.Name, in.Call.Arguments)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	if in.Call.Scope.Name == "get_ui_preference_options" {
		return result(in.Call.OperationID, assistant.AgentToolSucceeded, map[string]any{"ok": true, "options": assistant.UIPreferenceOptions(), "current_preferences_available": false, "currency_scope": "display_only", "preview_seconds": 8}, "service:"+in.Call.OperationID)
	}
	// Persisted by the ordinary operation transaction. The original deadline is
	// returned on every replay; no actor, credential digest or action enters the
	// tool result that Core may place in a model transcript.
	return result(in.Call.OperationID, assistant.AgentToolSucceeded, uiPreparation{true, "browser_action_prepared", false, s.now().Unix() + 120}, "service:"+in.Call.OperationID)
}
