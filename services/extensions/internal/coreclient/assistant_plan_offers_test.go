package coreclient

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/planoffers"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

const offersWire = `{"contract":"assistant.plan-offers.v1","actor_user_id":7,"checked_at_unix":123456,"topup_restricted":false,"plans":[{"id":1,"revision":2,"purchases":0,"external_prices":[],"terms":{"title":"Current plan","price_units":"100","duration_seconds":3600,"duration_months":0,"reset_period":"grant","reset_seconds":0,"limit_units":"1000","models":["*"],"groups":["*"],"max_purchases_per_user":0,"enabled":true}}],"topup_channels":[]}`

type offersPeer struct {
	factsPeer
	raw string
}

func (p *offersPeer) Capabilities(ctx context.Context, r *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	reply, err := p.factsPeer.Capabilities(ctx, r)
	if err == nil && p.enabled {
		reply.Features = append(reply.Features, planoffers.Feature)
	}
	return reply, err
}
func (p *offersPeer) GetAssistantBusinessFacts(ctx context.Context, r *pb.GetAssistantBusinessFactsRequest) (*pb.GetAssistantBusinessFactsResponse, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	if r.Kind != pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_PLAN_OFFERS {
		return nil, status.Error(codes.InvalidArgument, "fixed enum required")
	}
	raw := p.raw
	if raw == "" {
		raw = offersWire
	}
	return &pb.GetAssistantBusinessFactsResponse{FactsJson: []byte(raw)}, nil
}
func offersTestClient(t *testing.T, p *offersPeer) *AssistantClient {
	t.Helper()
	dir, err := os.MkdirTemp("", "offers-uds-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "rpc")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	pb.RegisterCoreControlServer(server, p)
	pb.RegisterCoreAssistantModelServer(server, p)
	go server.Serve(listener)
	t.Cleanup(server.Stop)
	control, err := New(socket, []byte(testToken))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { control.Close() })
	client, err := NewAssistantClient(socket, []byte(agentHost), control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	return client
}
func TestPlanOffersIndependentCapabilityAndOriginalSession(t *testing.T) {
	old := &factsPeer{enabled: true}
	oldClient := factsTestClient(t, old)
	if _, err := oldClient.GetAssistantPlanOffers(context.Background(), agentSession); status.Code(err) != codes.FailedPrecondition || old.calls != 0 || old.authCalls != 0 {
		t.Fatal(err, old.calls, old.authCalls)
	}
	peer := &offersPeer{factsPeer: factsPeer{enabled: true}}
	client := offersTestClient(t, peer)
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "x-lmm-user-credential", "evil"))
	facts, err := client.GetAssistantPlanOffers(ctx, agentSession)
	if err != nil || facts.ActorUserID != 7 || len(facts.Plans) != 1 || !facts.Plans[0].Terms.WalletAllowed() || peer.calls != 1 || peer.authCalls != 1 {
		t.Fatal(facts, err, peer.calls, peer.authCalls)
	}
}
func TestPlanOffersRejectsMalformedRequiredMoneyAndActor(t *testing.T) {
	variants := map[string]string{
		"actor":               strings.Replace(offersWire, `"actor_user_id":7`, `"actor_user_id":8`, 1),
		"restriction missing": strings.Replace(offersWire, `"topup_restricted":false,`, "", 1),
		"restriction null":    strings.Replace(offersWire, `"topup_restricted":false`, `"topup_restricted":null`, 1),
		"price missing":       strings.Replace(offersWire, `"price_units":"100",`, "", 1),
		"price null":          strings.Replace(offersWire, `"price_units":"100"`, `"price_units":null`, 1),
		"price floating":      strings.Replace(offersWire, `"price_units":"100"`, `"price_units":100.5`, 1),
		"count missing":       strings.Replace(offersWire, `"purchases":0,`, "", 1),
		"count null":          strings.Replace(offersWire, `"purchases":0`, `"purchases":null`, 1),
		"duplicate":           strings.Replace(offersWire, `"purchases":0`, `"purchases":1,"purchases":0`, 1),
		"authority":           strings.Replace(offersWire, `"purchases":0`, `"purchases":0,"payer":1`, 1),
	}
	for name, raw := range variants {
		t.Run(name, func(t *testing.T) {
			peer := &offersPeer{factsPeer: factsPeer{enabled: true}, raw: raw}
			client := offersTestClient(t, peer)
			if _, err := client.GetAssistantPlanOffers(context.Background(), agentSession); status.Code(err) != codes.DataLoss || peer.calls != 1 {
				t.Fatal(err, peer.calls)
			}
		})
	}
	for _, mode := range []string{"api-key", "team", "unknown-actor"} {
		t.Run(mode, func(t *testing.T) {
			peer := &offersPeer{factsPeer: factsPeer{enabled: true}}
			peer.mode = mode
			client := offersTestClient(t, peer)
			if _, err := client.GetAssistantPlanOffers(context.Background(), agentSession); err == nil || peer.calls != 0 {
				t.Fatal(err, peer.calls)
			}
		})
	}
}
