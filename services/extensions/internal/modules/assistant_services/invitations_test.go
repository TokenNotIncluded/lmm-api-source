package assistant_services

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/invitationfacts"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"strings"
	"testing"
	"time"
)

func invitationTestFacts(user int64) invitationfacts.Facts {
	code, email := "aB09", "owner@example.invalid"
	return invitationfacts.Facts{Version: 1, OK: true, Scope: "self", ActorUserID: user, AuthVersion: 2, CheckedAtUnix: time.Now().Unix(), InvitationCode: &code, ActorEmail: &email, NativeRegisteredReferrals: 0, AutomaticSignupRewardsReason: "registration_records_attribution_without_issuing_reward", BalanceStatus: "available", Balances: &invitationfacts.Balances{WalletSignedCredits: -3, AffiliateDebtCredits: 2, OpeningAffiliateHistoryCredits: 7, HistoryScope: "validated_opening_or_native_zero_basis_only"}, CreditUnit: "core_credit"}
}

type invitationTestReader struct{ facts invitationfacts.Facts }

func (r *invitationTestReader) InvitationFacts(context.Context, string) (invitationfacts.Facts, error) {
	v := r.facts
	v.CheckedAtUnix = time.Now().Unix()
	return v, nil
}

type invitationTestClient struct {
	facts       invitationfacts.Facts
	credentials []string
}

func (c *invitationTestClient) InvitationFactsReady(context.Context) error { return nil }
func (c *invitationTestClient) GetAssistantInvitationFacts(_ context.Context, s string) (invitationfacts.Facts, error) {
	c.credentials = append(c.credentials, s)
	return c.facts, nil
}
func TestInvitationReaderUsesOriginalSessionAndOmitsPrivateEmail(t *testing.T) {
	client := &invitationTestClient{facts: invitationTestFacts(101)}
	reader, e := NewCoreInvitations(client, pgServiceAuthority{})
	if e != nil {
		t.Fatal(e)
	}
	f, e := reader.InvitationFacts(context.Background(), "session-user-101")
	if e != nil {
		t.Fatal(e)
	}
	if len(client.credentials) != 1 || client.credentials[0] != "session-user-101" {
		t.Fatal("session replaced")
	}
	raw, _ := json.Marshal(invitationProjection(f))
	if strings.Contains(string(raw), "owner@example.invalid") || strings.Contains(string(raw), "actor_email") || strings.Contains(string(raw), "auth_version") {
		t.Fatal("private Core fields entered model output")
	}
	if _, e = reader.InvitationFacts(context.Background(), "revoked"); e != access.ErrUnauthorized || len(client.credentials) != 1 {
		t.Fatal("revoked session reached facts")
	}
	client.facts.ActorUserID = 202
	if _, e = reader.InvitationFacts(context.Background(), "session-user-101"); e != access.ErrUnavailable {
		t.Fatal("foreign facts accepted", e)
	}
}
func TestInvitationPayloadExactRecipientCodeAndNoCreation(t *testing.T) {
	f := invitationTestFacts(101)
	raw, e := invitationPayload(f, json.RawMessage(`{"email":" Visitor+tag@Example.Invalid "}`))
	if e != nil {
		t.Fatal(e)
	}
	if string(raw) != `{"kind":"invitation_email_v1","email":"visitor+tag@example.invalid","invitation_code":"aB09"}` {
		t.Fatal("recipient/code changed", string(raw))
	}
	for _, raw := range []string{`{"email":"owner@example.invalid"}`, `{"email":"Display <user@example.invalid>"}`, `{"email":"user@example.invalid","confirmed":true}`, `{"email":"bad\\r\\n@example.invalid"}`, `{"email":"user@example.invalid","invitation_code":"other"}`} {
		if _, e := invitationPayload(f, json.RawMessage(raw)); e == nil {
			t.Fatal("invalid invitation accepted", raw)
		}
	}
	f.InvitationCode = nil
	if _, e := invitationPayload(f, json.RawMessage(`{"email":"user@example.invalid"}`)); e != access.ErrConflict {
		t.Fatal("missing code silently created", e)
	}
}
func TestInvitationAvailableBindingsRequireActualPorts(t *testing.T) {
	has := func(s *Service, name string) bool {
		for _, b := range s.AvailableBindings() {
			if b.Name == name {
				return true
			}
		}
		return false
	}
	s := &Service{}
	if has(s, "send_invitation") || has(s, "get_invitation_rewards") {
		t.Fatal("unconfigured invitations advertised")
	}
	s.dependencies.Invitations = &invitationTestReader{facts: invitationTestFacts(101)}
	if !has(s, "get_invitation_rewards") || has(s, "send_invitation") {
		t.Fatal("read/send readiness not independent")
	}
	s.dependencies.InvitationMail = &invitationCountingSender{}
	if has(s, "send_invitation") {
		t.Fatal("human authority not required")
	}
	s.dependencies.Confirmations = &pgServiceConfirm{}
	if !has(s, "send_invitation") {
		t.Fatal("fully configured invitation not advertised")
	}
}

type invitationCountingSender struct{}

func (*invitationCountingSender) SendInvitation(context.Context, string, string, string) InvitationDelivery {
	return InvitationDelivery{"unknown", 0}
}
