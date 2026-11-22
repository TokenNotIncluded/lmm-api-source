package assistant

import (
	"context"
	"encoding/json"
	"slices"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// This is a browser-only envelope. It never enters ResultJSON, a provider
// transcript or history. The digest binds the exact, already authorized bearer;
// access-token refresh deliberately invalidates a pending display action.
type UIPreferenceAction struct {
	Type                  string            `json:"type"`
	Tool                  string            `json:"tool"`
	RequiresConfirmation  bool              `json:"requires_confirmation"`
	ActionID              string            `json:"action_id"`
	ActorUserID           int64             `json:"actor_user_id"`
	ActorCredentialSHA256 string            `json:"actor_credential_sha256"`
	ExpiresAt             int64             `json:"expires_at"`
	Temporary             bool              `json:"temporary"`
	Preview               map[string]string `json:"preview"`
}

// Implementations read the original durable domain operation and compare its
// Session and call digest. A caller-supplied result or model text is insufficient.
type UIPreferenceActionReader interface {
	CompletedUIPreferenceAction(context.Context, string, AgentToolCall, AgentToolReceipt) (*UIPreferenceAction, error)
}

var uiPreferenceOptions = map[string][]string{
	"mode":     {"light", "dark", "system"},
	"theme":    {"ant", "default", "simple-large", "underground", "rose-garden", "lake-view", "sunset-glow", "forest-whisper", "ocean-breeze", "lavender-dream"},
	"language": {"zh-CN", "en", "fr", "ru", "ja", "vi", "zh-TW"},
	"currency": {"auto", "USD", "CNY", "CREDIT"},
}

func UIPreferenceOptions() map[string][]string {
	out := make(map[string][]string, len(uiPreferenceOptions))
	for key, values := range uiPreferenceOptions {
		out[key] = slices.Clone(values)
	}
	return out
}

func IsUIPreferenceAction(name string) bool {
	return name == "set_ui_preferences" || name == "restore_ui_preferences"
}

// Only display data is accepted. In particular there is no actor, URL, CSS,
// script, arbitrary profile field, settlement currency or action ID argument.
func ParseUIPreferenceArguments(name string, raw json.RawMessage) (map[string]string, bool, error) {
	var input map[string]json.RawMessage
	if access.Decode(raw, &input) != nil {
		return nil, false, access.ErrInvalid
	}
	patch := map[string]string{}
	if name == "get_ui_preference_options" || name == "restore_ui_preferences" {
		if len(input) != 0 {
			return nil, false, access.ErrInvalid
		}
		return patch, false, nil
	}
	if name != "set_ui_preferences" {
		return nil, false, access.ErrInvalid
	}
	temporary := false
	for key, value := range input {
		if key == "temporary" {
			if string(value) == "null" || json.Unmarshal(value, &temporary) != nil {
				return nil, false, access.ErrInvalid
			}
			continue
		}
		choices, found := uiPreferenceOptions[key]
		var selected string
		if !found || json.Unmarshal(value, &selected) != nil {
			return nil, false, access.ErrInvalid
		}
		// Explicit old spelling aliases, with one canonical browser value.
		if key == "theme" && selected == "anthropic" {
			selected = "ant"
		}
		if key == "language" && selected == "zhCN" {
			selected = "zh-CN"
		}
		if key == "language" && selected == "zhTW" {
			selected = "zh-TW"
		}
		if !slices.Contains(choices, selected) {
			return nil, false, access.ErrInvalid
		}
		patch[key] = selected
	}
	if len(patch) == 0 || temporary && (patch["language"] != "" || patch["currency"] != "") {
		return nil, false, access.ErrInvalid
	}
	return patch, temporary, nil
}

func UIPreferenceBindings(executor AgentToolExecutor) []AgentToolBinding {
	properties := map[string]any{}
	for key, values := range UIPreferenceOptions() {
		if key == "theme" {
			values = append(values, "anthropic")
		}
		if key == "language" {
			values = append(values, "zhCN", "zhTW")
		}
		properties[key] = map[string]any{"type": "string", "enum": values}
	}
	properties["temporary"] = map[string]any{"type": "boolean", "description": "Preview only mode/theme for eight seconds without saving. Omitted or false saves the selected display fields."}
	empty := map[string]any{"type": "object", "additionalProperties": false, "properties": map[string]any{}}
	set := map[string]any{"type": "object", "additionalProperties": false, "properties": properties, "anyOf": []any{
		map[string]any{"required": []string{"mode"}}, map[string]any{"required": []string{"theme"}}, map[string]any{"required": []string{"language"}}, map[string]any{"required": []string{"currency"}},
	}}
	definitions := []struct {
		name, description, effect string
		schema                    any
	}{
		{"get_ui_preference_options", "List supported interface modes, themes, languages and balance display units. These are choices, not the current browser settings. Currency affects display only, never balances, rates or settlement.", "read_only", empty},
		{"set_ui_preferences", "Prepare only the current user's explicitly requested display changes for the active browser. Omitted fields stay unchanged. Use temporary=true only for an invited eight-second theme/mode preview; never loop or flash changes. Language and currency cannot be temporary. The browser reports application or failure and provides Undo. A prepared action is not proof of application, and cannot change billing or account authority.", "server_guarded", set},
		{"restore_ui_preferences", "On the user's request, stop the active appearance preview or undo this browser session's last assistant display change while preserving newer manual choices. This is not a factory reset. The browser may have nothing to restore after a reload; never invent success.", "server_guarded", empty},
	}
	out := make([]AgentToolBinding, 0, len(definitions))
	for _, d := range definitions {
		raw, err := json.Marshal(d.schema)
		if err != nil {
			panic("invalid display preference schema")
		}
		out = append(out, AgentToolBinding{Name: d.name, Description: d.description, Effect: d.effect, Access: "user", Version: FixedMainToolVersion, PriceVersion: BuiltinToolPriceVersion, PriceSHA256: FreeAgentPriceSHA256(), Parameters: raw, Executor: executor})
	}
	return out
}

func (s *AgentChat) WithUIPreferences(reader UIPreferenceActionReader) *AgentChat {
	copy := *s
	copy.uiPreferences = reader
	return &copy
}

// Only one exact successful Core-accepted operation may produce a side
// channel. Neither tentative output, history nor a same-shaped model object
// can reach this path. Ambiguous multiple display calls produce no action.
func (s *AgentChat) completedUIPreferenceAction(ctx context.Context, credential string, turn AgentStoredTurn) (*UIPreferenceAction, error) {
	if access.Missing(s.uiPreferences) || turn.Snapshot == nil || turn.Snapshot.Phase != AgentTurnCompleted {
		return nil, nil
	}
	var selected *AgentStoredTool
	for _, tool := range turn.Tools {
		if tool != nil && IsUIPreferenceAction(tool.Call.Scope.Name) {
			if selected != nil {
				return nil, nil
			}
			selected = tool
		}
	}
	if selected == nil || selected.Phase != "core_accepted" || selected.Result == nil || selected.Result.State != AgentToolSucceeded || selected.Receipt == nil || !validAgentToolReceipt(selected.Call, selected.ClaimKey, *selected.Result, *selected.Receipt) {
		return nil, nil
	}
	expected, err := s.registry.Scope(selected.Call.Scope.Name)
	if err != nil || selected.Call.Scope != expected {
		return nil, nil
	}
	p, err := s.actor(ctx, credential)
	if err != nil {
		return nil, err
	}
	if p.UserID != turn.Snapshot.OwnerUserID {
		return nil, access.ErrForbidden
	}
	action, err := s.uiPreferences.CompletedUIPreferenceAction(ctx, credential, selected.Call, *selected.Receipt)
	if err != nil || action == nil {
		return action, err
	}
	patch, temporary, err := ParseUIPreferenceArguments(selected.Call.Scope.Name, selected.Call.Arguments)
	now := time.Now().Unix()
	if err != nil || action.Type != "workspace_action" || action.Tool != selected.Call.Scope.Name || action.RequiresConfirmation || len(action.ActionID) != 32 || !lowerDigest(action.ActionID+action.ActionID) || action.ActorUserID != p.UserID || action.ActorCredentialSHA256 != agentDigest([]byte(credential)) || action.ExpiresAt <= now || action.ExpiresAt > now+120 || action.Temporary != temporary || !sameAgentJSON(action.Preview, patch) {
		return nil, access.ErrUnavailable
	}
	return action, nil
}
