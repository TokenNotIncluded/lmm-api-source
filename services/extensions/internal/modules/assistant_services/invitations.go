package assistant_services

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/invitationfacts"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"strings"
)

type InvitationFactsClient interface {
	InvitationFactsReady(context.Context) error
	GetAssistantInvitationFacts(context.Context, string) (invitationfacts.Facts, error)
}
type InvitationReader interface {
	InvitationFacts(context.Context, string) (invitationfacts.Facts, error)
}
type CoreInvitations struct {
	client InvitationFactsClient
	auth   access.Authority
}

func NewCoreInvitations(c InvitationFactsClient, a access.Authority) (*CoreInvitations, error) {
	if access.Missing(c) || access.Missing(a) {
		return nil, access.ErrInvalid
	}
	return &CoreInvitations{c, a}, nil
}
func (c *CoreInvitations) Ready(ctx context.Context) error { return c.client.InvitationFactsReady(ctx) }
func (c *CoreInvitations) InvitationFacts(ctx context.Context, credential string) (invitationfacts.Facts, error) {
	p, e := c.auth.Check(ctx, credential, access.Account{}, access.Read)
	if e != nil {
		return invitationfacts.Facts{}, e
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return invitationfacts.Facts{}, access.ErrForbidden
	}
	f, e := c.client.GetAssistantInvitationFacts(ctx, credential)
	if e != nil {
		return invitationfacts.Facts{}, access.Translate(e)
	}
	if !f.Valid(p.UserID) {
		return invitationfacts.Facts{}, access.ErrUnavailable
	}
	after, e := c.auth.Check(ctx, credential, access.Account{}, access.Read)
	if e != nil {
		return invitationfacts.Facts{}, e
	}
	if after.UserID != p.UserID || after.Account != p.Account {
		return invitationfacts.Facts{}, access.ErrForbidden
	}
	return f, nil
}

type invitationDraft struct {
	Kind           string `json:"kind"`
	Email          string `json:"email"`
	InvitationCode string `json:"invitation_code"`
}

func (s *Service) invitationFacts(ctx context.Context, credential string, p access.Principal) (invitationfacts.Facts, error) {
	if access.Missing(s.dependencies.Invitations) {
		return invitationfacts.Facts{}, access.ErrUnavailable
	}
	f, e := s.dependencies.Invitations.InvitationFacts(ctx, credential)
	if e != nil {
		return f, e
	}
	if !f.Valid(p.UserID) {
		return f, access.ErrUnavailable
	}
	return f, nil
}
func invitationProjection(f invitationfacts.Facts) any {
	return map[string]any{
		"ok": true, "scope": "self", "invitation_code": f.InvitationCode,
		"invitation_code_created": false, "native_registered_referrals": f.NativeRegisteredReferrals,
		"legacy_invited_count_available": false, "legacy_invited_count": nil,
		"automatic_signup_rewards_enabled": false, "automatic_signup_rewards_reason": f.AutomaticSignupRewardsReason,
		"balance_status": f.BalanceStatus, "balances": f.Balances, "credit_unit": f.CreditUnit,
		"financial_authority": false, "checked_at_unix": f.CheckedAtUnix,
	}
}
func invitationDefinitions() []assistantOpenAIToolDefinition {
	return []assistantOpenAIToolDefinition{
		{Type: "function", Function: PureDefinition{Name: "get_invitation_rewards", Description: "Read your existing invitation code and current Core referral balance. Reading never creates a code, invitation, or reward. Historical invite counts may be unavailable; native registration counts are explicitly separate. The current registration path records attribution without automatically issuing new signup rewards. Gross opening history is not current spendable balance.", Parameters: emptyObjectSchema()}},
		{Type: "function", Function: PureDefinition{Name: "send_invitation", Description: "Prepare one invitation email to a bare email address using your existing invitation code. The browser must confirm the exact recipient and code. A model call never sends mail. After one dispatch, timeout or unknown result is not retried. SMTP acceptance is not proof of delivery. Do not use this for bulk mail or invite yourself.", Parameters: objectSchema(map[string]any{"email": map[string]any{"type": "string", "minLength": 3, "maxLength": 254}}, []string{"email"})}},
	}
}
func (s *Service) invitationRead(ctx context.Context, in assistant.AgentInvocation, p access.Principal) (any, error) {
	var input struct{}
	if decode(in.Call.Arguments, &input) != nil {
		return nil, access.ErrInvalid
	}
	f, e := s.invitationFacts(ctx, in.Credential, p)
	if e != nil {
		return nil, e
	}
	return invitationProjection(f), nil
}
func invitationPayload(f invitationfacts.Facts, raw json.RawMessage) (json.RawMessage, error) {
	var in struct {
		Email string `json:"email"`
	}
	if decode(raw, &in) != nil {
		return nil, access.ErrInvalid
	}
	address, e := normalizeInvitationEmail(in.Email)
	if e != nil {
		return nil, e
	}
	if f.InvitationCode == nil || !invitationfacts.ValidCode(*f.InvitationCode) {
		return nil, access.ErrConflict
	}
	if f.ActorEmail != nil && strings.ToLower(strings.TrimSpace(*f.ActorEmail)) == address {
		return nil, access.ErrConflict
	}
	return json.Marshal(invitationDraft{Kind: "invitation_email_v1", Email: address, InvitationCode: *f.InvitationCode})
}

func (s *Service) invitationTool(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal, session [32]byte) (assistant.AgentToolResult, bool, error) {
	switch in.Call.Scope.Name {
	case "get_invitation_rewards":
		value, err := s.invitationRead(ctx, in, p)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		out, err := result(in.Call.OperationID, assistant.AgentToolSucceeded, value, "service:"+in.Call.OperationID)
		return out, true, err
	case "send_invitation":
		if access.Missing(s.dependencies.InvitationMail) || access.Missing(s.dependencies.Confirmations) {
			return assistant.AgentToolResult{}, true, access.ErrUnavailable
		}
		facts, err := s.invitationFacts(ctx, in.Credential, p)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		payload, err := invitationPayload(facts, in.Call.Arguments)
		if err != nil {
			return assistant.AgentToolResult{}, true, err
		}
		out, err := s.prepareInTx(ctx, tx, in, p, session, payload)
		return out, true, err
	default:
		return assistant.AgentToolResult{}, false, nil
	}
}
func rejectInvitationDatabaseApply(context.Context, *sql.Tx, access.Principal, json.RawMessage) (any, error) {
	// SMTP is exclusively in confirmInvitationLocked after a durable ACK.
	return nil, access.ErrUnavailable
}
