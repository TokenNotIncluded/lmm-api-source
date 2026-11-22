package coreclient

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/invitationfacts"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

func invitationWire() string {
	return fmt.Sprintf(`{"version":1,"ok":true,"scope":"self","actor_user_id":7,"auth_version":2,"checked_at_unix":%d,"invitation_code":"aB12","actor_email":"owner@example.invalid","native_registered_referrals":0,"legacy_invited_count_available":false,"legacy_invited_count":null,"automatic_signup_rewards_enabled":false,"automatic_signup_rewards_reason":"registration_records_attribution_without_issuing_reward","balance_status":"available","balances":{"wallet_signed_credits":-3,"affiliate_available_credits":0,"affiliate_debt_credits":2,"opening_affiliate_history_credits":5,"history_scope":"validated_opening_or_native_zero_basis_only"},"credit_unit":"core_credit","financial_authority":false}`, time.Now().Unix())
}

type invitationsPeer struct {
	factsPeer
	raw string
}

func (p *invitationsPeer) Capabilities(ctx context.Context, r *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	reply, e := p.factsPeer.Capabilities(ctx, r)
	if e == nil && p.enabled {
		reply.Features = []string{invitationfacts.Feature}
	}
	return reply, e
}
func (p *invitationsPeer) GetAssistantBusinessFacts(ctx context.Context, r *pb.GetAssistantBusinessFactsRequest) (*pb.GetAssistantBusinessFactsResponse, error) {
	if e := p.check(ctx); e != nil {
		return nil, e
	}
	if r.Kind != pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_INVITATION_REWARDS || r.GetTargetUserId() != 0 {
		return nil, status.Error(codes.InvalidArgument, "fixed self-only enum required")
	}
	if p.mode == "lost" {
		return nil, status.Error(codes.Unavailable, "synthetic unavailable")
	}
	raw := p.raw
	if raw == "" {
		raw = invitationWire()
	}
	out := &pb.GetAssistantBusinessFactsResponse{FactsJson: []byte(raw)}
	if p.mode == "unknown" {
		out.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	}
	return out, nil
}
func invitationsClient(t *testing.T, p *invitationsPeer) *AssistantClient {
	t.Helper()
	dir, e := os.MkdirTemp("", "invitation-uds-")
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "rpc")
	l, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	server := grpc.NewServer()
	pb.RegisterCoreControlServer(server, p)
	pb.RegisterCoreAssistantModelServer(server, p)
	go server.Serve(l)
	t.Cleanup(server.Stop)
	control, e := New(socket, []byte(testToken))
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(func() { control.Close() })
	client, e := NewAssistantClient(socket, []byte(agentHost), control)
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(func() { client.Close() })
	return client
}
func TestInvitationClientIndependentCapabilityAndOriginalSession(t *testing.T) {
	old := &factsPeer{enabled: true}
	oldClient := factsTestClient(t, old)
	if _, e := oldClient.GetAssistantInvitationFacts(context.Background(), agentSession); status.Code(e) != codes.FailedPrecondition || old.calls != 0 || old.authCalls != 0 {
		t.Fatal(e, old.calls, old.authCalls)
	}
	p := &invitationsPeer{factsPeer: factsPeer{enabled: true}}
	c := invitationsClient(t, p)
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "x-lmm-user-credential", "evil"))
	f, e := c.GetAssistantInvitationFacts(ctx, agentSession)
	if e != nil || f.ActorUserID != 7 || f.InvitationCode == nil || *f.InvitationCode != "aB12" || f.Balances == nil || f.Balances.AffiliateDebtCredits != 2 || p.calls != 1 || p.authCalls != 1 {
		t.Fatal(e, p.calls, p.authCalls)
	}
}
func TestInvitationClientRejectsMalformedOrSubstitutedAuthority(t *testing.T) {
	wire := invitationWire()
	variants := map[string]string{
		"actor":           strings.Replace(wire, `"actor_user_id":7`, `"actor_user_id":8`, 1),
		"missing":         strings.Replace(wire, `"affiliate_debt_credits":2,`, "", 1),
		"null":            strings.Replace(wire, `"affiliate_debt_credits":2`, `"affiliate_debt_credits":null`, 1),
		"fraction":        strings.Replace(wire, `"affiliate_debt_credits":2`, `"affiliate_debt_credits":2.5`, 1),
		"duplicate":       strings.Replace(wire, `"actor_user_id":7`, `"actor_user_id":7,"actor_user_id":7`, 1),
		"unknown":         strings.Replace(wire, `"financial_authority":false`, `"financial_authority":false,"payer":1`, 1),
		"invented_reward": strings.Replace(wire, `"automatic_signup_rewards_enabled":false`, `"automatic_signup_rewards_enabled":true`, 1),
		"stale":           strings.Replace(wire, fmt.Sprintf(`"checked_at_unix":%d`, time.Now().Unix()), `"checked_at_unix":1`, 1),
	}
	for name, raw := range variants {
		t.Run(name, func(t *testing.T) {
			p := &invitationsPeer{factsPeer: factsPeer{enabled: true}, raw: raw}
			c := invitationsClient(t, p)
			if _, e := c.GetAssistantInvitationFacts(context.Background(), agentSession); status.Code(e) != codes.DataLoss || p.calls != 1 {
				t.Fatal(e, p.calls)
			}
		})
	}
	for _, mode := range []string{"api-key", "team", "unknown-actor", "unknown", "lost"} {
		t.Run(mode, func(t *testing.T) {
			p := &invitationsPeer{factsPeer: factsPeer{enabled: true}}
			p.mode = mode
			c := invitationsClient(t, p)
			_, e := c.GetAssistantInvitationFacts(context.Background(), agentSession)
			if e == nil || p.authCalls != 1 || p.calls > 1 {
				t.Fatal(e, p.authCalls, p.calls)
			}
			if (mode == "api-key" || mode == "team" || mode == "unknown-actor") && p.calls != 0 {
				t.Fatal("invalid actor reached RPC")
			}
		})
	}
}
