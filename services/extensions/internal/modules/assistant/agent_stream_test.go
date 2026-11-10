// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"encoding/hex"
	"errors"
	"io"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// The durable journal and Core outcomes are controlled fixtures. These tests
// prove Go orchestration, not real Core execution, billing or PostgreSQL.
type agentStreamFixture struct {
	*agentChatPortFixture
	watches, cancels int
	readyErr         error
	watchErr         error
}

func (p *agentStreamFixture) StreamReady(context.Context) error { return p.readyErr }
func (p *agentStreamFixture) WatchAgentOutput(ctx context.Context, token string, key AgentTurnKey, owner int64, digest []byte, consume func(AgentOutput) error) error {
	p.watches++
	if owner != 1 || agentDigest([]byte(agentRequest.Message)) != hex.EncodeToString(digest) || key != p.snapshot.Key || token != "session" {
		return access.ErrForbidden
	}
	for i, text := range []string{"真实", "真实输出"} {
		if err := consume(AgentOutput{Sequence: uint64(i + 1), Phase: "model", Text: text}); err != nil {
			return err
		}
	}
	if p.watchErr != nil {
		return p.watchErr
	}
	p.snapshot.Phase, p.snapshot.FinalText = AgentTurnCompleted, "Core final answer"
	p.snapshot.Revision++
	return consume(AgentOutput{Sequence: 3, Phase: "end"})
}
func (p *agentStreamFixture) CancelAgentTurn(_ context.Context, _ string, key AgentTurnKey) (AgentTurnSnapshot, error) {
	p.cancels++
	if key != p.snapshot.Key {
		return AgentTurnSnapshot{}, access.ErrConflict
	}
	p.snapshot.Phase = AgentTurnUnknown
	p.snapshot.Revision++
	return p.snapshot, nil
}

func TestAgentStreamOriginalTurnOutputAndCompletion(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	p.snapshot.Phase, p.snapshot.PendingTools = AgentTurnRunning, nil
	stream := &agentStreamFixture{agentChatPortFixture: p}
	c.port = stream
	var frames []AgentOutput
	reply, err := c.StreamChat(context.Background(), "session", agentRequest, func(frame AgentOutput) error {
		if h.calls != 0 || frame.ConversationID != 10 || frame.ClientTurnID != "turn" {
			t.Fatal("tentative output established history or lost binding")
		}
		frames = append(frames, frame)
		return nil
	})
	if err != nil || reply.State != "settled" || len(frames) != 3 || frames[1].Text != "真实输出" || p.starts != 1 || e.executes != 0 || h.calls != 1 {
		t.Fatal(reply, err, frames, p.starts, h.calls)
	}
}

func TestAgentStreamDisconnectCancelAndReadOnlyReconnect(t *testing.T) {
	c, _, p, e, h := agentChatFixture(t)
	p.snapshot.Phase, p.snapshot.PendingTools, p.snapshot.HeldCredits = AgentTurnRunning, nil, 10
	stream := &agentStreamFixture{agentChatPortFixture: p}
	c.port = stream
	_, err := c.StreamChat(context.Background(), "session", agentRequest, func(AgentOutput) error { return io.ErrClosedPipe })
	if !errors.Is(err, io.ErrClosedPipe) || p.starts != 1 || stream.cancels != 0 || h.calls != 0 {
		t.Fatal(err, p.starts, stream.cancels)
	}
	reply, err := c.Cancel(context.Background(), "session", 10, "turn")
	if err != nil || reply.State != "unknown" || p.snapshot.HeldCredits != 10 || stream.cancels != 1 {
		t.Fatal(reply, err)
	}
	reply, err = c.WatchOutput(context.Background(), "session", 10, "turn", func(AgentOutput) error { t.Fatal("cancelled operation supplied live output"); return nil })
	if err != nil || reply.State != "unknown" || p.starts != 1 || stream.watches != 1 || e.executes != 0 || h.calls != 0 {
		t.Fatal(reply, err)
	}
	if _, err := c.Cancel(context.Background(), "session", 11, "turn"); !errors.Is(err, access.ErrNotFound) || stream.cancels != 1 {
		t.Fatal("cross-owner mapping passed", err)
	}
}

func TestAgentStreamMissingCapabilityDoesNotStart(t *testing.T) {
	c, _, p, _, _ := agentChatFixture(t)
	c.port = &agentStreamFixture{agentChatPortFixture: p, readyErr: ErrDelegationUnavailable}
	if _, err := c.StreamChat(context.Background(), "session", agentRequest, func(AgentOutput) error { return nil }); !errors.Is(err, ErrDelegationUnavailable) || p.starts != 0 {
		t.Fatal(err, p.starts)
	}
}
