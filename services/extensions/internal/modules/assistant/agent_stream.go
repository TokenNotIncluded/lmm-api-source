// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"encoding/hex"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// AgentOutput is a replaceable, tentative snapshot, not a settlement receipt.
// Sequence belongs to one live observer; reconnect reads the current prefix.
type AgentOutput struct {
	ConversationID int64  `json:"conversation_id"`
	ClientTurnID   string `json:"client_turn_id"`
	Sequence       uint64 `json:"sequence"`
	Round          uint32 `json:"round"`
	Phase          string `json:"phase"`
	Text           string `json:"text"`
}

type AgentOutputPort interface {
	StreamReady(context.Context) error
	WatchAgentOutput(context.Context, string, AgentTurnKey, int64, []byte, func(AgentOutput) error) error
	CancelAgentTurn(context.Context, string, AgentTurnKey) (AgentTurnSnapshot, error)
}

func (s *AgentChat) StreamReady(ctx context.Context) error {
	p, ok := s.port.(AgentOutputPort)
	if !ok || access.Missing(p) {
		return ErrDelegationUnavailable
	}
	return p.StreamReady(ctx)
}

func (s *AgentChat) StreamChat(ctx context.Context, credential string, request PersonalChatRequest, consume func(AgentOutput) error) (PersonalChatReply, error) {
	if consume == nil {
		return PersonalChatReply{}, access.ErrInvalid
	}
	if err := s.StreamReady(ctx); err != nil {
		return PersonalChatReply{}, err
	}
	ctx, cancel := context.WithTimeout(ctx, 300*time.Second)
	defer cancel()
	return s.chat(ctx, credential, request, consume)
}

func (s *AgentChat) outputTurn(ctx context.Context, credential string, conversation int64, turn string) (access.Principal, AgentStoredTurn, AgentTurnKey, error) {
	if !validAgentChatID(conversation, turn) {
		return access.Principal{}, AgentStoredTurn{}, AgentTurnKey{}, access.ErrInvalid
	}
	p, err := s.actor(ctx, credential)
	var stored AgentStoredTurn
	var key AgentTurnKey
	if err == nil {
		err = s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
			if local.Turn == nil || local.Mapping == nil || local.Mapping.Phase != "registered" {
				return access.ErrNotFound
			}
			key = AgentTurnKey{local.Mapping.CoreConversationID, turn}
			stored = *local.Turn
			return nil
		})
	}
	return p, stored, key, err
}

func (s *AgentChat) watchSnapshot(ctx context.Context, credential string, p access.Principal, conversation int64, turn string, stored AgentStoredTurn, consume func(AgentOutput) error) error {
	port, ok := s.port.(AgentOutputPort)
	if !ok {
		return ErrDelegationUnavailable
	}
	digest, err := hex.DecodeString(stored.InputSHA256)
	if err != nil || len(digest) != 32 || stored.Snapshot == nil {
		return ErrDelegationUnknown
	}
	return port.WatchAgentOutput(ctx, credential, stored.Snapshot.Key, p.UserID, digest, func(event AgentOutput) error {
		event.ConversationID, event.ClientTurnID = conversation, turn
		event.Text = RedactAssistantHistoryContent(event.Text)
		return consume(event)
	})
}

// WatchOutput reconnects to an existing turn without starting a model or tool.
// A missing live slot is not a successful stream; use Recover for its receipt.
func (s *AgentChat) WatchOutput(ctx context.Context, credential string, conversation int64, turn string, consume func(AgentOutput) error) (PersonalChatReply, error) {
	if consume == nil {
		return PersonalChatReply{}, access.ErrInvalid
	}
	if err := s.StreamReady(ctx); err != nil {
		return PersonalChatReply{}, err
	}
	p, stored, key, err := s.outputTurn(ctx, credential, conversation, turn)
	if err != nil {
		return PersonalChatReply{}, err
	}
	snap, err := s.port.GetAgentTurn(ctx, credential, key)
	if err != nil {
		return PersonalChatReply{}, err
	}
	stored, err = s.storeSnapshot(ctx, p, conversation, turn, snap)
	if err != nil {
		return PersonalChatReply{}, err
	}
	if snap.Phase == AgentTurnRunning {
		if err = s.watchSnapshot(ctx, credential, p, conversation, turn, stored, consume); err != nil {
			return PersonalChatReply{}, err
		}
	}
	return s.Recover(ctx, credential, conversation, turn)
}

func (s *AgentChat) Cancel(ctx context.Context, credential string, conversation int64, turn string) (PersonalChatReply, error) {
	port, ok := s.port.(AgentOutputPort)
	if !ok {
		return PersonalChatReply{}, ErrDelegationUnavailable
	}
	p, _, key, err := s.outputTurn(ctx, credential, conversation, turn)
	if err != nil {
		return PersonalChatReply{}, err
	}
	snap, err := port.CancelAgentTurn(ctx, credential, key)
	if err != nil {
		return PersonalChatReply{}, agentRecoveryError(err)
	}
	return s.advance(ctx, credential, p, conversation, turn, snap, true, time.Now().Add(15*time.Second), nil)
}
