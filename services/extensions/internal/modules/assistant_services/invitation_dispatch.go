package assistant_services

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type invitationConfirmation struct {
	id, operation, version, challenge, payloadSHA, coreDraftSHA string
	level                                                       int32
	user, expires                                               int64
	session, payload, savedResult                               []byte
	consumed                                                    sql.NullInt64
}

func invitationUnknown(operation, id string) json.RawMessage {
	raw, _ := json.Marshal(map[string]any{
		"ok": false, "applied": false, "status": "unknown", "smtp_accepted": false,
		"delivery_confirmed": false, "retry_safe": false,
		"operation_id": operation, "confirmation_id": id,
	})
	return raw
}

// The caller already holds the actual user's advisory lock and the immutable
// confirmation row. A committed marker has no takeover path: only the process
// which received the first commit acknowledgement can attempt SMTP.
func (s *Service) confirmInvitationLocked(ctx context.Context, tx *sql.Tx, p access.Principal, credential string, c invitationConfirmation) (json.RawMessage, error) {
	if c.consumed.Valid {
		return append(json.RawMessage(nil), c.savedResult...), nil
	}
	unknown := invitationUnknown(c.operation, c.id)
	var opUser int64
	var opSession, call []byte
	var tool string
	err := tx.QueryRowContext(ctx, `SELECT user_id,session_sha256,call_sha256,tool_name FROM lmm_assistant.service_operations WHERE operation_id=$1`, c.operation).Scan(&opUser, &opSession, &call, &tool)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if opUser != p.UserID || tool != "send_invitation" || !bytes.Equal(opSession, c.session) || len(call) != 32 {
		return nil, access.ErrConflict
	}
	var markerUser int64
	var markerSession, markerCall []byte
	err = tx.QueryRowContext(ctx, `SELECT user_id,session_sha256,call_sha256 FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, c.operation).Scan(&markerUser, &markerSession, &markerCall)
	if err == nil {
		if markerUser != p.UserID || !bytes.Equal(markerSession, c.session) || !bytes.Equal(markerCall, call) {
			return nil, access.ErrConflict
		}
		// Do not consume the confirmation here. The first sender may still be in I/O.
		return unknown, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return nil, access.ErrUnavailable
	}
	if access.Missing(s.dependencies.InvitationMail) || access.Missing(s.dependencies.Invitations) {
		return nil, access.ErrUnavailable
	}
	if c.expires <= s.now().Unix() || c.version != assistant.FixedMainToolVersion || c.level != 0 {
		return nil, access.ErrConflict
	}
	if digest(c.payload) != c.payloadSHA {
		return nil, access.ErrUnavailable
	}
	var draft invitationDraft
	if decode(c.payload, &draft) != nil || draft.Kind != "invitation_email_v1" {
		return nil, access.ErrUnavailable
	}
	recipient, err := normalizeInvitationEmail(draft.Email)
	if err != nil || recipient != draft.Email {
		return nil, access.ErrUnavailable
	}
	facts, err := s.invitationFacts(ctx, credential, p)
	if err != nil {
		return nil, err
	}
	expected, err := invitationPayload(facts, json.RawMessage(mustInvitationInput(recipient)))
	if err != nil {
		return nil, err
	}
	if !bytes.Equal(expected, c.payload) {
		return nil, access.ErrConflict
	}
	proof, err := s.dependencies.Confirmations.Verify(ctx, credential, c.challenge, c.operation, c.coreDraftSHA)
	if err != nil {
		return nil, err
	}
	if !proof.Confirmed || proof.UserID != p.UserID || proof.OperationID != c.operation || proof.PayloadSHA256 != c.coreDraftSHA || proof.ReceiptID == "" || proof.ExpiresAt <= s.now().Unix() {
		return nil, access.ErrForbidden
	}
	current, err := s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	if current.UserID != p.UserID || current.Account != p.Account {
		return nil, access.ErrForbidden
	}
	var now, count int64
	if err = tx.QueryRowContext(ctx, `SELECT floor(extract(epoch FROM clock_timestamp()))::bigint`).Scan(&now); err != nil {
		return nil, access.ErrUnavailable
	}
	// The shared Core assistant-workspace bucket applies to browser decisions.
	// This additional invitation-only durable limit is not a replacement for it.
	err = tx.QueryRowContext(ctx, `SELECT count(*) FROM lmm_assistant.service_network_dispatch d JOIN lmm_assistant.service_confirmations c USING(operation_id) WHERE d.user_id=$1 AND c.tool_name='send_invitation' AND d.created_at>$2-300`, p.UserID, now).Scan(&count)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if count >= 120 {
		return nil, access.ErrLimit
	}
	if _, err = tx.ExecContext(ctx, `SET LOCAL synchronous_commit=on`); err != nil {
		return nil, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.service_network_dispatch(operation_id,user_id,session_sha256,call_sha256,created_at) VALUES($1,$2,$3,$4,$5)`, c.operation, p.UserID, c.session, call, now); err != nil {
		return nil, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		// Even a later read of this marker cannot prove this process obtained ACK.
		// This invocation never sends and a future invocation never takes it over.
		return unknown, nil
	}

	delivery := s.dependencies.InvitationMail.SendInvitation(ctx, c.operation, recipient, draft.InvitationCode)
	switch delivery.State {
	case "accepted_by_smtp":
		if delivery.SMTPCode != 250 {
			delivery = InvitationDelivery{"unknown", 0}
		}
	case "rejected":
		if delivery.SMTPCode < 400 || delivery.SMTPCode >= 600 {
			delivery = InvitationDelivery{"unknown", 0}
		}
	case "not_dispatched", "unknown":
		if delivery.SMTPCode != 0 {
			delivery = InvitationDelivery{"unknown", 0}
		}
	default:
		delivery = InvitationDelivery{"unknown", 0}
	}
	accepted := delivery.State == "accepted_by_smtp"
	raw, err := json.Marshal(map[string]any{
		"ok": accepted, "applied": accepted, "status": delivery.State,
		"smtp_accepted": accepted, "smtp_code": delivery.SMTPCode,
		"delivery_confirmed": false, "retry_safe": false,
		"operation_id": c.operation, "confirmation_id": c.id,
		"proof_receipt_id": proof.ReceiptID,
	})
	if err != nil {
		return unknown, nil
	}
	// Persist the first sender's outcome even if the browser has disconnected.
	// It is an audit of an already authorized attempt, not permission for new I/O.
	finish, cancel := context.WithTimeout(context.WithoutCancel(ctx), 5*time.Second)
	defer cancel()
	saved, err := s.finishInvitation(finish, c, call, raw)
	if err != nil {
		saved, err = s.readInvitationOutcome(finish, c)
		if err != nil {
			saved = unknown
		}
	}
	// Current authorization still controls disclosure after the external attempt.
	current, authErr := s.principal(finish, credential)
	if authErr != nil {
		return nil, authErr
	}
	if current.UserID != p.UserID || current.Account != p.Account {
		return nil, access.ErrForbidden
	}
	return saved, nil
}

func mustInvitationInput(recipient string) []byte {
	raw, _ := json.Marshal(struct {
		Email string `json:"email"`
	}{recipient})
	return raw
}

func (s *Service) finishInvitation(ctx context.Context, c invitationConfirmation, call, raw []byte) (json.RawMessage, error) {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SET LOCAL synchronous_commit=on`); err != nil {
		return nil, err
	}
	var saved []byte
	err = tx.QueryRowContext(ctx, `UPDATE lmm_assistant.service_confirmations c
 SET consumed_at=floor(extract(epoch FROM clock_timestamp()))::bigint,result=$6
 WHERE c.id=$1 AND c.operation_id=$2 AND c.user_id=$3 AND c.session_sha256=$4 AND c.tool_name='send_invitation' AND c.consumed_at IS NULL
 AND EXISTS(SELECT FROM lmm_assistant.service_network_dispatch d WHERE d.operation_id=c.operation_id AND d.user_id=c.user_id AND d.session_sha256=c.session_sha256 AND d.call_sha256=$5)
 RETURNING result`, c.id, c.operation, c.user, c.session, call, string(raw)).Scan(&saved)
	if err != nil {
		return nil, err
	}
	if err = tx.Commit(); err != nil {
		return nil, err
	}
	return json.RawMessage(saved), nil
}

func (s *Service) readInvitationOutcome(ctx context.Context, c invitationConfirmation) (json.RawMessage, error) {
	var saved []byte
	err := s.db.QueryRowContext(ctx, `SELECT result FROM lmm_assistant.service_confirmations WHERE id=$1 AND operation_id=$2 AND user_id=$3 AND session_sha256=$4 AND tool_name='send_invitation' AND consumed_at IS NOT NULL`, c.id, c.operation, c.user, c.session).Scan(&saved)
	return json.RawMessage(saved), err
}
