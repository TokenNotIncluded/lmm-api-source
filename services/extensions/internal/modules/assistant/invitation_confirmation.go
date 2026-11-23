package assistant

import (
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"time"
)

// This is a private browser response field, never a model tool result or history.
type InvitationConfirmationAction struct {
	Type                  string `json:"type"`
	Tool                  string `json:"tool"`
	ConfirmationID        string `json:"confirmation_id"`
	OperationID           string `json:"operation_id"`
	ActorUserID           int64  `json:"actor_user_id"`
	ActorCredentialSHA256 string `json:"actor_credential_sha256"`
	ExpiresAt             int64  `json:"expires_at"`
	Recipient             string `json:"recipient"`
}
type InvitationConfirmationReader interface {
	PendingInvitationConfirmation(context.Context, string, AgentToolCall, AgentToolResult) (*InvitationConfirmationAction, error)
}

func (s *AgentChat) WithInvitationConfirmations(reader InvitationConfirmationReader) *AgentChat {
	copy := *s
	copy.invitationConfirmations = reader
	return &copy
}
func (s *AgentChat) pendingInvitationConfirmation(ctx context.Context, credential string, p access.Principal, conversation int64, turnID string) (*InvitationConfirmationAction, error) {
	if access.Missing(s.invitationConfirmations) {
		return nil, nil
	}
	var turn AgentStoredTurn
	err := s.journal.WithinAgent(ctx, p.UserID, conversation, turnID, func(local *AgentLocalState) error {
		if local.Turn == nil || local.Mapping == nil {
			return access.ErrNotFound
		}
		turn = *local.Turn
		return nil
	})
	if err != nil {
		return nil, err
	}
	if turn.Snapshot == nil || turn.Snapshot.Phase != AgentTurnAwaitingTools || turn.Snapshot.OwnerUserID != p.UserID {
		return nil, nil
	}
	var selected *AgentStoredTool
	for _, call := range turn.Snapshot.PendingTools {
		if call.Scope.Name != "send_invitation" {
			continue
		}
		if selected != nil {
			return nil, nil
		}
		tool := turn.Tools[call.OperationID]
		if tool == nil || !sameAgentJSON(tool.Call, call) || tool.Phase != "result_stored" || tool.Result == nil || tool.Result.State != AgentToolPending || tool.Receipt != nil || !pendingCreationKey(tool.ClaimKey) {
			return nil, nil
		}
		expected, e := s.registry.Scope(call.Scope.Name)
		if e != nil || call.Scope != expected {
			return nil, nil
		}
		selected = tool
	}
	if selected == nil {
		return nil, nil
	}
	current, err := s.actor(ctx, credential)
	if err != nil {
		return nil, err
	}
	if current.UserID != p.UserID || current.Account != p.Account {
		return nil, access.ErrForbidden
	}
	action, err := s.invitationConfirmations.PendingInvitationConfirmation(ctx, credential, selected.Call, *selected.Result)
	if err != nil || action == nil {
		return action, err
	}
	now := time.Now().Unix()
	if action.Type != "invitation_confirmation" || action.Tool != "send_invitation" || !lowerDigest(action.ConfirmationID) || action.OperationID != selected.Call.OperationID || action.ActorUserID != p.UserID || action.ActorCredentialSHA256 != agentDigest([]byte(credential)) || action.ExpiresAt <= now || action.ExpiresAt > now+300 || len(action.Recipient) == 0 || len(action.Recipient) > 254 {
		return nil, access.ErrUnavailable
	}
	return action, nil
}
