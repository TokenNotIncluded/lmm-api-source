package coreclient

import (
	"context"
	"encoding/json"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/adminskillfacts"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type adminSkillsPeer struct {
	factsPeer
	raw string
}

func adminSkillsWire() string {
	raw, _ := json.Marshal(adminskillfacts.Authorization{Version: 1, ActorUserID: 7, TargetUserID: 202, ActorAuthVersion: 3, TargetAuthVersion: 5, ActorRole: "admin", TargetRole: "user", AuthorizationBindingSHA256: strings.Repeat("a", 64), CheckedAtUnix: time.Now().Unix()})
	return string(raw)
}
func (p *adminSkillsPeer) Capabilities(ctx context.Context, r *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	reply, err := p.factsPeer.Capabilities(ctx, r)
	if err == nil && p.enabled {
		reply.Features = []string{adminskillfacts.Feature}
	}
	return reply, err
}
func (p *adminSkillsPeer) GetAssistantBusinessFacts(ctx context.Context, r *pb.GetAssistantBusinessFactsRequest) (*pb.GetAssistantBusinessFactsResponse, error) {
	if err := p.check(ctx); err != nil {
		return nil, err
	}
	if r.Kind != pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_ADMIN_USER_SKILLS_AUTHORIZATION || r.GetTargetUserId() != 202 {
		return nil, status.Error(codes.InvalidArgument, "exact administrator authorization kind/target required")
	}
	if p.mode == "lost" {
		return nil, status.Error(codes.Unavailable, "synthetic unavailable")
	}
	raw := p.raw
	if raw == "" {
		raw = adminSkillsWire()
	}
	out := &pb.GetAssistantBusinessFactsResponse{FactsJson: []byte(raw)}
	if p.mode == "unknown" {
		out.ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
	}
	return out, nil
}
func adminSkillsClient(t *testing.T, p *adminSkillsPeer) *AssistantClient {
	t.Helper()
	dir, err := os.MkdirTemp("", "admin-skills-uds-")
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
func TestAdminSkillsClientRequiresDedicatedCapabilityAndOriginalSession(t *testing.T) {
	old := &factsPeer{enabled: true}
	oldClient := factsTestClient(t, old)
	if _, err := oldClient.GetAssistantAdminSkillAuthorization(context.Background(), agentSession, 202); status.Code(err) != codes.FailedPrecondition || old.calls != 0 || old.authCalls != 0 {
		t.Fatal(err, old.calls, old.authCalls)
	}
	peer := &adminSkillsPeer{factsPeer: factsPeer{enabled: true}}
	client := adminSkillsClient(t, peer)
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "substituted", "x-lmm-user-credential", "substituted"))
	out, err := client.GetAssistantAdminSkillAuthorization(ctx, agentSession, 202)
	if err != nil || out.ActorUserID != 7 || out.TargetUserID != 202 || peer.calls != 1 || peer.authCalls != 1 {
		t.Fatal(err, out, peer.calls, peer.authCalls)
	}
}
func TestAdminSkillsClientRejectsSubstitutedAndMalformedFacts(t *testing.T) {
	wire := adminSkillsWire()
	variants := map[string]string{
		"actor":              strings.Replace(wire, `"actor_user_id":7`, `"actor_user_id":8`, 1),
		"target":             strings.Replace(wire, `"target_user_id":202`, `"target_user_id":203`, 1),
		"user_role":          strings.Replace(wire, `"actor_role":"admin"`, `"actor_role":"user"`, 1),
		"same_rank":          strings.Replace(wire, `"target_role":"user"`, `"target_role":"admin"`, 1),
		"zero_generation":    strings.Replace(wire, `"actor_auth_version":3`, `"actor_auth_version":0`, 1),
		"missing_generation": strings.Replace(wire, `"target_auth_version":5,`, "", 1),
		"null_generation":    strings.Replace(wire, `"target_auth_version":5`, `"target_auth_version":null`, 1),
		"duplicate":          strings.Replace(wire, `"actor_user_id":7`, `"actor_user_id":7,"actor_user_id":7`, 1),
		"unknown":            strings.Replace(wire, `"version":1`, `"version":1,"grant":true`, 1),
		"bad_digest":         strings.Replace(wire, strings.Repeat("a", 64), strings.Repeat("A", 64), 1),
		"oversized":          strings.Replace(wire, `"actor_role":"admin"`, `"actor_role":"`+strings.Repeat("a", 4096)+`"`, 1),
		"trailing":           wire + "{}",
	}
	for name, raw := range variants {
		t.Run(name, func(t *testing.T) {
			peer := &adminSkillsPeer{factsPeer: factsPeer{enabled: true}, raw: raw}
			if _, err := adminSkillsClient(t, peer).GetAssistantAdminSkillAuthorization(context.Background(), agentSession, 202); status.Code(err) != codes.DataLoss {
				t.Fatal("bad fact accepted", err)
			}
		})
	}
	for _, mode := range []string{"api-key", "team", "unknown-actor", "unknown", "lost"} {
		t.Run(mode, func(t *testing.T) {
			peer := &adminSkillsPeer{factsPeer: factsPeer{enabled: true}}
			peer.mode = mode
			_, err := adminSkillsClient(t, peer).GetAssistantAdminSkillAuthorization(context.Background(), agentSession, 202)
			expected := codes.PermissionDenied
			if mode == "unknown" {
				expected = codes.DataLoss
			}
			if mode == "lost" {
				expected = codes.Unavailable
			}
			if status.Code(err) != expected {
				t.Fatal(mode, err)
			}
			if (mode == "api-key" || mode == "team" || mode == "unknown-actor") && peer.calls != 0 {
				t.Fatal("fact read before valid Session")
			}
		})
	}
}
