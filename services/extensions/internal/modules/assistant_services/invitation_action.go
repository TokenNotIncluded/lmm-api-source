package assistant_services

import (
	"context"
	"database/sql"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// Only an authenticated, currently pending original operation can expose a
// confirmation reference. No model result selects a URL or proves consent.
func (s *Service) PendingInvitationConfirmation(ctx context.Context, credential string, call assistant.AgentToolCall, pending assistant.AgentToolResult) (*assistant.InvitationConfirmationAction, error) {
	if call.Scope.Name != "send_invitation" {
		return nil, access.ErrInvalid
	}
	registry, e := assistant.NewAgentRegistry(s.AvailableBindings())
	if e != nil {
		return nil, e
	}
	expected, e := registry.Scope("send_invitation")
	if e != nil || call.Scope != expected {
		return nil, access.ErrConflict
	}
	p, e := s.principal(ctx, credential)
	if e != nil {
		return nil, e
	}
	in := assistant.AgentInvocation{Credential: credential, Actor: p, Call: call}
	_, session, callHash, e := s.validate(ctx, in)
	if e != nil {
		return nil, e
	}
	tx, e := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if e != nil {
		return nil, access.ErrUnavailable
	}
	defer tx.Rollback()
	saved, e := s.readOperation(ctx, tx, in, p, session, callHash)
	if errors.Is(e, access.ErrConflict) || errors.Is(e, access.ErrNotFound) {
		return nil, nil
	}
	if e != nil {
		return nil, e
	}
	if saved.State != assistant.AgentToolPending {
		return nil, nil
	}
	if pending.OperationID != call.OperationID || pending.State != assistant.AgentToolPending || pending.ResultSHA256 != saved.ResultSHA256 || pending.DomainReceiptID != saved.DomainReceiptID {
		return nil, access.ErrConflict
	}
	id := digest([]byte("assistant-confirmation:" + call.OperationID))
	var expires int64
	var payload []byte
	var marker bool
	var consumed sql.NullInt64
	e = tx.QueryRowContext(ctx, `SELECT payload_bytes,expires_at,consumed_at,EXISTS(SELECT FROM lmm_assistant.service_network_dispatch d WHERE d.operation_id=c.operation_id) FROM lmm_assistant.service_confirmations c WHERE id=$1 AND operation_id=$2 AND user_id=$3 AND session_sha256=$4 AND tool_name='send_invitation' AND tool_version=$5`, id, call.OperationID, p.UserID, session[:], assistant.FixedMainToolVersion).Scan(&payload, &expires, &consumed, &marker)
	if errors.Is(e, sql.ErrNoRows) {
		return nil, access.ErrNotFound
	}
	if e != nil {
		return nil, access.ErrUnavailable
	}
	if marker || consumed.Valid || expires <= s.now().Unix() {
		return nil, nil
	}
	var draft struct {
		Kind  string `json:"kind"`
		Email string `json:"email"`
		Code  string `json:"invitation_code"`
	}
	if decode(payload, &draft) != nil || draft.Kind != "invitation_email_v1" {
		return nil, access.ErrUnavailable
	}
	normalized, e := normalizeInvitationEmail(draft.Email)
	if e != nil || normalized != draft.Email {
		return nil, access.ErrUnavailable
	}
	// Validate the source argument against the saved normalized recipient again.
	var source struct {
		Email string `json:"email"`
	}
	if decode(call.Arguments, &source) != nil {
		return nil, access.ErrConflict
	}
	recipient, e := normalizeInvitationEmail(source.Email)
	if e != nil || recipient != draft.Email {
		return nil, access.ErrConflict
	}
	if e = tx.Commit(); e != nil {
		return nil, access.ErrUnavailable
	}
	return &assistant.InvitationConfirmationAction{Type: "invitation_confirmation", Tool: "send_invitation", ConfirmationID: id, OperationID: call.OperationID, ActorUserID: p.UserID, ActorCredentialSHA256: digest([]byte(credential)), ExpiresAt: expires, Recipient: recipient}, nil
}
