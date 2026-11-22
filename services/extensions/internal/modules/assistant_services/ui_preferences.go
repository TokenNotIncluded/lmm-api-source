package assistant_services

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

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

func (s *Service) CompletedUIPreferenceAction(ctx context.Context, credential string, call assistant.AgentToolCall, receipt assistant.AgentToolReceipt) (*assistant.UIPreferenceAction, error) {
	if !assistant.IsUIPreferenceAction(call.Scope.Name) {
		return nil, access.ErrInvalid
	}
	registry, err := assistant.NewAgentRegistry(assistant.UIPreferenceBindings(s))
	if err != nil {
		return nil, err
	}
	expected, err := registry.Scope(call.Scope.Name)
	if err != nil || call.Scope != expected {
		return nil, access.ErrConflict
	}
	p, err := s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	in := assistant.AgentInvocation{Credential: credential, Actor: p, Call: call}
	_, session, callHash, err := s.validate(ctx, in)
	if err != nil {
		return nil, err
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer tx.Rollback()
	saved, err := s.readOperation(ctx, tx, in, p, session, callHash)
	// A refresh or another login cannot rebind an old action. The settled chat
	// can still be shown, but it carries no executable display envelope.
	if errors.Is(err, access.ErrConflict) || errors.Is(err, access.ErrNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if saved.State != assistant.AgentToolSucceeded || receipt.OperationID != call.OperationID || receipt.State != assistant.AgentToolSucceeded || receipt.ResultSHA256 != saved.ResultSHA256 || receipt.DomainReceiptID != saved.DomainReceiptID || saved.DomainReceiptID != "service:"+call.OperationID {
		return nil, access.ErrConflict
	}
	var prepared uiPreparation
	var keys map[string]json.RawMessage
	if decode(saved.ResultJSON, &prepared) != nil || decode(saved.ResultJSON, &keys) != nil || len(keys) != 4 || !prepared.OK || prepared.Applied || prepared.Status != "browser_action_prepared" {
		return nil, access.ErrUnavailable
	}
	now := s.now().Unix()
	if prepared.ExpiresAt <= now {
		return nil, nil
	}
	if prepared.ExpiresAt > now+120 {
		return nil, access.ErrUnavailable
	}
	patch, temporary, err := assistant.ParseUIPreferenceArguments(call.Scope.Name, call.Arguments)
	if err != nil {
		return nil, err
	}
	return &assistant.UIPreferenceAction{Type: "workspace_action", Tool: call.Scope.Name, ActionID: digest([]byte("assistant.ui-preference.v1\x00" + call.OperationID + "\x00" + saved.ResultSHA256))[:32], ActorUserID: p.UserID, ActorCredentialSHA256: digest([]byte(credential)), ExpiresAt: prepared.ExpiresAt, Temporary: temporary, Preview: patch}, nil
}
