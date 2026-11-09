package coreclient

import (
	"context"
	"crypto/sha256"
	"errors"
	"net"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

type outputPeer struct {
	pb.UnimplementedCoreControlServer
	pb.UnimplementedCoreAssistantModelServer
	features []string
	events   []*pb.AssistantOutputEvent
	mode     string
	started  chan struct{}
	release  chan struct{}
	closed   chan struct{}
	watches  atomic.Int32
	cancels  atomic.Int32
}

func (p *outputPeer) Capabilities(ctx context.Context, _ *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	md, _ := metadata.FromIncomingContext(ctx)
	if len(md.Get("x-lmm-user-credential")) != 0 || len(md.Get("authorization")) != 1 || md.Get("authorization")[0] != "Bearer "+testToken {
		return nil, status.Error(codes.Unauthenticated, "control metadata")
	}
	return &pb.CapabilitiesResponse{ProtocolMajor: 1, Features: p.features}, nil
}
func outputMetadata(ctx context.Context) error {
	md, _ := metadata.FromIncomingContext(ctx)
	for key, want := range map[string]string{"authorization": "Bearer " + agentHost, "x-lmm-protocol": "1", "x-lmm-user-credential": agentSession} {
		if got := md.Get(key); len(got) != 1 || got[0] != want {
			return status.Error(codes.Unauthenticated, "assistant metadata")
		}
	}
	return nil
}
func (p *outputPeer) WatchAssistantOutput(_ *pb.AssistantOutputLookup, stream grpc.ServerStreamingServer[pb.AssistantOutputEvent]) error {
	p.watches.Add(1)
	defer close(p.closed)
	if err := outputMetadata(stream.Context()); err != nil {
		return err
	}
	close(p.started)
	for i, event := range p.events {
		if err := stream.Send(event); err != nil {
			return err
		}
		if i == 0 && p.release != nil {
			select {
			case <-p.release:
			case <-stream.Context().Done():
				return stream.Context().Err()
			}
		}
	}
	if p.mode == "revoked" {
		return status.Error(codes.PermissionDenied, "original grant revoked")
	}
	return nil
}
func (p *outputPeer) CancelAgentTurn(ctx context.Context, r *pb.AssistantAgentLookup) (*pb.AssistantAgentTurn, error) {
	p.cancels.Add(1)
	if err := outputMetadata(ctx); err != nil {
		return nil, err
	}
	if p.mode == "lost-cancel" {
		return nil, status.Error(codes.Unavailable, "uncertain acknowledgement")
	}
	out := agentTurnFixture(r.GetCoreConversationId(), r.GetClientTurnId(), "hello")
	out.Phase = pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_UNKNOWN
	out.ModelHeldCredits = 10
	return out, nil
}
func outputTestClient(t *testing.T, p *outputPeer) *AssistantClient {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer(grpc.MaxRecvMsgSize(MaxMessage), grpc.MaxSendMsgSize(MaxMessage))
	pb.RegisterCoreControlServer(server, p)
	pb.RegisterCoreAssistantModelServer(server, p)
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(server.Stop)
	client := func(token string) *Client {
		c := &Client{authorization: "Bearer " + token, slots: make(chan struct{}, maxInFlight)}
		conn, err := grpc.NewClient("passthrough:///"+listener.Addr().String(), grpc.WithTransportCredentials(insecure.NewCredentials()), grpc.WithDisableRetry(), grpc.WithDisableServiceConfig(), grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(MaxMessage), grpc.MaxCallSendMsgSize(MaxMessage)), grpc.WithUnaryInterceptor(c.boundCall))
		if err != nil {
			t.Fatal(err)
		}
		c.connection, c.rpc = conn, pb.NewCoreControlClient(conn)
		t.Cleanup(func() { _ = conn.Close() })
		return c
	}
	return &AssistantClient{rpc: client(agentHost), control: client(testToken)}
}
func outputFixture(personal bool) (*pb.AssistantOutputLookup, []byte, []*pb.AssistantOutputEvent) {
	h := sha256.Sum256([]byte("hello"))
	cv := "lmm_cv_" + strings.Repeat("c", 64)
	target := &pb.AssistantOutputLookup{Target: &pb.AssistantOutputLookup_AgentTurn{AgentTurn: &pb.AssistantAgentLookup{CoreConversationId: cv, ClientTurnId: "turn"}}}
	if personal {
		target.Target = &pb.AssistantOutputLookup_PersonalText{PersonalText: &pb.AssistantOperationBinding{DelegationId: "lmm_ad_" + strings.Repeat("a", 64), CoreConversationId: cv, TurnId: "turn", OperationKey: "turn:0", TextSha256: h[:]}}
	}
	var events []*pb.AssistantOutputEvent
	for i, text := range []string{"你好", "你好，world", ""} {
		phase := pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_MODEL
		if i == 2 {
			phase = pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_END
		}
		events = append(events, &pb.AssistantOutputEvent{Target: proto.Clone(target).(*pb.AssistantOutputLookup), Sequence: uint64(3 + i*2), Text: text, Phase: phase, OwnerUserId: 1, OriginalTextSha256: h[:]})
	}
	return target, h[:], events
}
func newOutputPeer(events []*pb.AssistantOutputEvent) *outputPeer {
	return &outputPeer{features: []string{AssistantPersonalTextReserveFeature, AssistantPersonalTextExecuteFeature, AssistantPersonalTextStreamFeature, AssistantAgentTurnFeature, AssistantAgentStreamFeature}, events: events, started: make(chan struct{}), closed: make(chan struct{})}
}
func TestAssistantOutputTCPBeforeCompletionAndCancellation(t *testing.T) {
	for _, personal := range []bool{false, true} {
		t.Run(map[bool]string{false: "agent", true: "personal"}[personal], func(t *testing.T) {
			target, digest, events := outputFixture(personal)
			p := newOutputPeer(events)
			p.release = make(chan struct{})
			c := outputTestClient(t, p)
			ctx, cancel := context.WithTimeout(metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "forged", "x-lmm-user-credential", "forged")), 5*time.Second)
			defer cancel()
			first := make(chan struct{})
			result := make(chan error, 1)
			go func() {
				result <- c.AssistantWatchOutput(ctx, agentSession, target, 1, digest, func(e *pb.AssistantOutputEvent) error {
					if e.Sequence == 3 {
						close(first)
					}
					return nil
				})
			}()
			select {
			case <-first:
			case err := <-result:
				t.Fatalf("no live prefix: %v", err)
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
			select {
			case <-p.closed:
				t.Fatal("prefix arrived only after completion")
			default:
			}
			cancel()
			if err := <-result; status.Code(err) != codes.Canceled {
				t.Fatalf("cancel: %v", err)
			}
			select {
			case <-p.closed:
			case <-time.After(time.Second):
				t.Fatal("observation connection survived cancellation")
			}
			if len(c.rpc.slots) != 0 || p.watches.Load() != 1 || p.cancels.Load() != 0 {
				t.Fatal("observation leaked admission or invoked cancellation authority")
			}
		})
	}
}
func TestAssistantOutputTCPRejectsCorruptionAndIncompleteEOF(t *testing.T) {
	mutations := map[string]func(*outputPeer){
		"ok":      func(*outputPeer) {},
		"binding": func(p *outputPeer) { p.events[0].Target.GetAgentTurn().ClientTurnId = "other" },
		"owner":   func(p *outputPeer) { p.events[0].OwnerUserId = 2 },
		"digest":  func(p *outputPeer) { p.events[0].OriginalTextSha256 = make([]byte, 32) },
		"unknown": func(p *outputPeer) { p.events[0].ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1}) },
		"nested-unknown": func(p *outputPeer) {
			p.events[0].Target.GetAgentTurn().ProtoReflect().SetUnknown([]byte{0x98, 0x06, 1})
		},
		"enum":      func(p *outputPeer) { p.events[0].Phase = 99 },
		"sequence":  func(p *outputPeer) { p.events[1].Sequence = p.events[0].Sequence },
		"round":     func(p *outputPeer) { p.events[0].Round = 32 },
		"prefix":    func(p *outputPeer) { p.events[1].Text = "different" },
		"tool-text": func(p *outputPeer) { p.events[0].Phase = pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_TOOL },
		"large":     func(p *outputPeer) { p.events[0].Text = strings.Repeat("x", 32769) },
		"end-text":  func(p *outputPeer) { p.events[2].Text = "not a receipt" },
		"after-end": func(p *outputPeer) { p.events = append(p.events, p.events[1]) },
		"early-eof": func(p *outputPeer) { p.events = p.events[:2] },
		"empty-eof": func(p *outputPeer) { p.events = nil },
		"revoked":   func(p *outputPeer) { p.events = p.events[:1]; p.mode = "revoked" },
	}
	for name, mutate := range mutations {
		t.Run(name, func(t *testing.T) {
			target, digest, events := outputFixture(false)
			p := newOutputPeer(events)
			mutate(p)
			c := outputTestClient(t, p)
			err := c.AssistantWatchOutput(context.Background(), agentSession, target, 1, digest, func(*pb.AssistantOutputEvent) error { return nil })
			want := codes.DataLoss
			if name == "ok" {
				want = codes.OK
			}
			if name == "revoked" {
				want = codes.PermissionDenied
			}
			if status.Code(err) != want || p.watches.Load() != 1 {
				t.Fatal(err, p.watches.Load())
			}
		})
	}
}
func TestAssistantOutputTCPCapabilityAndOriginalCancel(t *testing.T) {
	target, digest, events := outputFixture(false)
	p := newOutputPeer(events)
	p.features = []string{AssistantAgentTurnFeature}
	c := outputTestClient(t, p)
	if err := c.AssistantWatchOutput(context.Background(), agentSession, target, 1, digest, func(*pb.AssistantOutputEvent) error { return nil }); status.Code(err) != codes.FailedPrecondition || p.watches.Load() != 0 {
		t.Fatal(err)
	}
	if _, err := c.AssistantCancelAgentTurn(context.Background(), agentSession, target.GetAgentTurn()); status.Code(err) != codes.FailedPrecondition || p.cancels.Load() != 0 {
		t.Fatal(err)
	}
	for _, mode := range []string{"", "lost-cancel"} {
		p := newOutputPeer(nil)
		p.mode = mode
		c := outputTestClient(t, p)
		out, err := c.AssistantCancelAgentTurn(context.Background(), agentSession, target.GetAgentTurn())
		if mode == "" && (err != nil || out.GetModelHeldCredits() != 10 || out.GetPhase() != pb.AssistantAgentPhase_ASSISTANT_AGENT_PHASE_UNKNOWN) {
			t.Fatal(out, err)
		}
		if mode != "" && status.Code(err) != codes.Unavailable {
			t.Fatal(err)
		}
		if p.cancels.Load() != 1 {
			t.Fatal("cancel retried uncertain mutation")
		}
	}
	p = newOutputPeer(events)
	c = outputTestClient(t, p)
	sinkErr := errors.New("HTTP write failed")
	if err := c.AssistantWatchOutput(context.Background(), agentSession, target, 1, digest, func(*pb.AssistantOutputEvent) error { return sinkErr }); !errors.Is(err, sinkErr) {
		t.Fatal(err)
	}
}
