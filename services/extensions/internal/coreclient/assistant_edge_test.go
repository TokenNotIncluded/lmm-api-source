package coreclient

import (
	"context"
	"crypto/sha256"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/edgecontext"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

const syntheticEdgeProof = "synthetic-edge-proof-for-private-uds-1111111111"

// This peer checks real HTTP -> context -> UDS metadata, not Core funds or PG.
type edgePeer struct {
	pb.UnimplementedCoreControlServer
	pb.UnimplementedCoreAssistantModelServer
	want  bool
	calls int
}

func (p *edgePeer) check(ctx context.Context, edge bool) error {
	m, _ := metadata.FromIncomingContext(ctx)
	if edge && p.want {
		if v := m.Get("x-lmm-edge-proof"); len(v) != 1 || v[0] != syntheticEdgeProof {
			return status.Error(codes.PermissionDenied, "wrong proof carrier")
		}
		if v := m.Get("x-lmm-edge-client-ip"); len(v) != 1 || v[0] != "203.0.113.7" {
			return status.Error(codes.PermissionDenied, "wrong client carrier")
		}
	} else if len(m.Get("x-lmm-edge-proof")) != 0 || len(m.Get("x-lmm-edge-client-ip")) != 0 {
		return status.Error(codes.PermissionDenied, "unexpected edge metadata")
	}
	if v := m.Get("authorization"); len(v) != 1 || v[0] != "Bearer "+agentHost {
		return status.Error(codes.Unauthenticated, "wrong service credential")
	}
	if v := m.Get("x-lmm-user-credential"); len(v) != 1 || v[0] != agentSession {
		return status.Error(codes.Unauthenticated, "wrong original Session")
	}
	p.calls++
	return nil
}
func (p *edgePeer) Capabilities(ctx context.Context, _ *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	m, _ := metadata.FromIncomingContext(ctx)
	if len(m.Get("x-lmm-edge-proof")) != 0 || len(m.Get("x-lmm-edge-client-ip")) != 0 || len(m.Get("x-lmm-user-credential")) != 0 {
		return nil, status.Error(codes.PermissionDenied, "discovery leaked request assertion")
	}
	return &pb.CapabilitiesResponse{ProtocolMajor: 1, Features: []string{AssistantPersonalTextReserveFeature, AssistantPersonalTextExecuteFeature, AssistantAgentTurnFeature, AssistantNewUserGiftFeature}}, nil
}
func (p *edgePeer) ReserveText(ctx context.Context, _ *pb.AssistantTextReservation) (*pb.AssistantOperationReceipt, error) {
	return &pb.AssistantOperationReceipt{}, p.check(ctx, true)
}
func (p *edgePeer) StartAgentTurn(ctx context.Context, r *pb.AssistantAgentStart) (*pb.AssistantAgentTurn, error) {
	return agentTurnFixture(r.CoreConversationId, r.ClientTurnId, r.Text), p.check(ctx, true)
}
func (p *edgePeer) GetNewUserGift(ctx context.Context, _ *pb.AssistantGiftRequest) (*pb.AssistantGiftStatus, error) {
	return &pb.AssistantGiftStatus{}, p.check(ctx, true)
}
func (p *edgePeer) ClaimStoredNewUserGift(ctx context.Context, _ *pb.AssistantGiftRequest) (*pb.AssistantGiftClaim, error) {
	return &pb.AssistantGiftClaim{}, p.check(ctx, true)
}
func (p *edgePeer) BeginConversation(ctx context.Context, _ *pb.AssistantConversationKey) (*pb.AssistantConversationReceipt, error) {
	return &pb.AssistantConversationReceipt{}, p.check(ctx, false)
}

func TestAssistantEdgeOnlyFourCallsCarryListenerAssertionOverUDS(t *testing.T) {
	dir, err := os.MkdirTemp("", "lmm-edge-uds-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	socket := filepath.Join(dir, "rpc")
	l, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	p := &edgePeer{}
	server := grpc.NewServer()
	pb.RegisterCoreControlServer(server, p)
	pb.RegisterCoreAssistantModelServer(server, p)
	go server.Serve(l)
	t.Cleanup(server.Stop)
	control, err := New(socket, []byte(testToken))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = control.Close() })
	client, err := NewAssistantClient(socket, []byte(agentHost), control)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = client.Close() })
	web := httptest.NewServer(edgecontext.Capture(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get(edgecontext.ProofHeader) != "" || r.Header.Get(edgecontext.ClientHeader) != "" {
			http.Error(w, "private header reached handler", 500)
			return
		}
		// Outgoing caller metadata is untrusted even inside a captured context.
		ctx := metadata.NewOutgoingContext(r.Context(), metadata.Pairs("x-lmm-edge-proof", "forged", "x-lmm-edge-client-ip", "192.0.2.9", "authorization", "forged"))
		h := sha256.Sum256([]byte("hello"))
		b := &pb.AssistantOperationBinding{DelegationId: "lmm_ad_" + strings.Repeat("a", 64), CoreConversationId: "lmm_cv_" + strings.Repeat("c", 64), TurnId: "edge-turn", OperationKey: "edge-turn:0", TextSha256: h[:]}
		checks := []func() error{
			func() error {
				_, e := client.AssistantReserveText(ctx, agentSession, &pb.AssistantTextReservation{Binding: b, Text: "hello"})
				return e
			},
			func() error {
				_, e := client.AssistantStartAgentTurn(ctx, agentSession, &pb.AssistantAgentStart{CoreConversationId: b.CoreConversationId, ClientTurnId: b.TurnId, Text: "hello"})
				return e
			},
			func() error { _, e := client.AssistantGetNewUserGift(ctx, agentSession); return e },
			func() error { _, e := client.AssistantClaimStoredNewUserGift(ctx, agentSession); return e },
			func() error {
				_, e := client.AssistantBeginConversation(ctx, agentSession, &pb.AssistantConversationKey{CreationKey: "edge-test"})
				return e
			},
		}
		for _, check := range checks {
			if e := check(); e != nil {
				http.Error(w, e.Error(), 500)
				return
			}
		}
		w.WriteHeader(http.StatusNoContent)
	})))
	t.Cleanup(web.Close)
	for _, withHeaders := range []bool{true, false} {
		p.want = withHeaders
		r, err := http.NewRequest("POST", web.URL, nil)
		if err != nil {
			t.Fatal(err)
		}
		if withHeaders {
			r.Header.Set(edgecontext.ProofHeader, syntheticEdgeProof)
			r.Header.Set(edgecontext.ClientHeader, "::ffff:203.0.113.7")
		}
		response, err := web.Client().Do(r)
		if err != nil {
			t.Fatal(err)
		}
		_ = response.Body.Close()
		if response.StatusCode != http.StatusNoContent {
			t.Fatal("HTTP/UDS assertion boundary failed", response.StatusCode)
		}
	}
	if p.calls != 10 {
		t.Fatal("unexpected call count", p.calls)
	}
}
