package coreclient

import (
	"bytes"
	"context"
	"crypto/sha256"
	"io"
	"strings"
	"time"
	"unicode/utf8"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

const AssistantPersonalTextStreamFeature = "assistant.personal_text.stream.v1"
const AssistantAgentStreamFeature = "assistant.agent_turn.stream.v1"

func (c *AssistantClient) PersonalTextStreamReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, AssistantPersonalTextReserveFeature, AssistantPersonalTextExecuteFeature, AssistantPersonalTextStreamFeature)
}
func (c *AssistantClient) AgentStreamReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, AssistantAgentTurnFeature, AssistantAgentStreamFeature)
}

func assistantOutputLookup(r *pb.AssistantOutputLookup) bool {
	if !assistantMessage(r) {
		return false
	}
	switch target := r.GetTarget().(type) {
	case *pb.AssistantOutputLookup_PersonalText:
		return assistantBinding(target.PersonalText)
	case *pb.AssistantOutputLookup_AgentTurn:
		return assistantMessage(target.AgentTurn) && agentKey(target.AgentTurn.GetCoreConversationId(), target.AgentTurn.GetClientTurnId())
	default:
		return false
	}
}

// AssistantWatchOutput only observes an existing original operation. A snapshot
// is tentative prose, never a usage, settlement, tool-claim or history receipt.
// The caller supplies identity and input digest from its authenticated durable
// turn; accepting a first frame must not establish either of those bindings.
func (c *AssistantClient) AssistantWatchOutput(ctx context.Context, user string, r *pb.AssistantOutputLookup, owner int64, input []byte, consume func(*pb.AssistantOutputEvent) error) error {
	if !assistantOutputLookup(r) || owner <= 0 || len(input) != sha256.Size || consume == nil {
		return agentInvalid()
	}
	if !validUserCredential(user) {
		return status.Error(codes.Unauthenticated, "invalid user credential")
	}
	personal := r.GetPersonalText() != nil
	if personal && !bytes.Equal(input, r.GetPersonalText().GetTextSha256()) {
		return agentInvalid()
	}
	var err error
	if personal {
		err = c.PersonalTextStreamReady(ctx)
	} else {
		err = c.AgentStreamReady(ctx)
	}
	if err != nil {
		return err
	}
	// Streaming RPCs do not pass the unary interceptor. Hold the same bounded
	// admission slot until EOF/cancellation and impose the worker's maximum age.
	select {
	case c.rpc.slots <- struct{}{}:
		defer func() { <-c.rpc.slots }()
	default:
		return status.Error(codes.ResourceExhausted, "core RPC client is busy")
	}
	bounded, cancel := context.WithTimeout(ctx, 300*time.Second)
	defer cancel()
	stream, err := pb.NewCoreAssistantModelClient(c.rpc.connection).WatchAssistantOutput(c.rpc.context(bounded, user), r)
	if err != nil {
		return err
	}
	var previous *pb.AssistantOutputEvent
	ended := false
	for {
		event, err := stream.Recv()
		if err == io.EOF {
			if !ended {
				return status.Error(codes.DataLoss, "assistant output ended without a terminal observation")
			}
			return nil
		}
		if err != nil {
			return err
		}
		if ended || !assistantMessage(event) || !proto.Equal(r, event.GetTarget()) || event.GetOwnerUserId() != owner || !bytes.Equal(input, event.GetOriginalTextSha256()) || event.GetSequence() == 0 || event.GetRound() >= 32 || len(event.GetText()) > 32768 || !utf8.ValidString(event.GetText()) {
			return agentCorrupt()
		}
		if personal && event.GetRound() != r.GetPersonalText().GetRound() {
			return agentCorrupt()
		}
		switch event.GetPhase() {
		case pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_MODEL:
		case pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_TOOL:
			if event.GetText() != "" {
				return agentCorrupt()
			}
		case pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_END:
			if event.GetText() != "" {
				return agentCorrupt()
			}
			ended = true
		default:
			return agentCorrupt()
		}
		if previous != nil {
			if event.GetSequence() <= previous.GetSequence() || event.GetRound() < previous.GetRound() {
				return agentCorrupt()
			}
			if event.GetRound() == previous.GetRound() && event.GetPhase() == pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_MODEL && (previous.GetPhase() != pb.AssistantOutputPhase_ASSISTANT_OUTPUT_PHASE_MODEL || !strings.HasPrefix(event.GetText(), previous.GetText())) {
				return agentCorrupt()
			}
		}
		previous = proto.Clone(event).(*pb.AssistantOutputEvent)
		if err := consume(event); err != nil {
			return err
		}
	}
}

// Cancellation never creates a replacement turn. A completed turn returns its
// original receipt; an uncertain dispatched model retains its existing hold.
func (c *AssistantClient) AssistantCancelAgentTurn(ctx context.Context, user string, r *pb.AssistantAgentLookup) (*pb.AssistantAgentTurn, error) {
	if !assistantMessage(r) || !agentKey(r.GetCoreConversationId(), r.GetClientTurnId()) {
		return nil, agentInvalid()
	}
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	if err := c.AgentStreamReady(ctx); err != nil {
		return nil, err
	}
	out, err := pb.NewCoreAssistantModelClient(c.rpc.connection).CancelAgentTurn(c.rpc.context(ctx, user), r)
	if err != nil {
		return nil, err
	}
	if !agentTurn(out, r.GetCoreConversationId(), r.GetClientTurnId()) {
		return nil, agentCorrupt()
	}
	return out, nil
}
