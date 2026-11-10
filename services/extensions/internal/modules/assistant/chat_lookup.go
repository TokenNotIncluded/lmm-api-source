// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// ChatConversationMapping identifies only an already persisted local creation
// claim. Phase describes registration, never model execution or settlement.
type ChatConversationMapping struct {
	ConversationID int64  `json:"conversation_id"`
	ClientTurnID   string `json:"client_turn_id"`
	Phase          string `json:"phase"`
}

func personalChatCreationKey(turn string) string {
	sum := sha256.Sum256([]byte("personal-chat-v1:" + turn))
	return hex.EncodeToString(sum[:16])
}

// LookupChatConversation is strictly read-only, including when the first chat's
// response/ready frame was lost. Recover is intentionally not used: it may bind
// a Core receipt locally. Neither a miss nor an inflight claim authorizes retry.
func (p *PendingConversations) LookupChatConversation(ctx context.Context, credential, turn string) (ChatConversationMapping, error) {
	if !access.ID(turn) || len(turn) > 64 {
		return ChatConversationMapping{}, access.ErrInvalid
	}
	owner, err := p.owner(ctx, credential)
	if err != nil {
		return ChatConversationMapping{}, err
	}
	local, err := p.repo.LookupPending(ctx, owner, personalChatCreationKey(turn))
	if err != nil {
		return ChatConversationMapping{}, err
	}
	if !validPendingConversation(local) || local.CreationKey != personalChatCreationKey(turn) {
		return ChatConversationMapping{}, access.ErrUnavailable
	}
	return ChatConversationMapping{ConversationID: local.ID, ClientTurnID: turn, Phase: local.Phase}, nil
}

func (s *AgentChat) LookupChatConversation(ctx context.Context, credential, turn string) (ChatConversationMapping, error) {
	return s.pending.LookupChatConversation(ctx, credential, turn)
}
func (s *PersonalChat) LookupChatConversation(ctx context.Context, credential, turn string) (ChatConversationMapping, error) {
	return s.pending.LookupChatConversation(ctx, credential, turn)
}

// RecoverChatConversation copies only an existing original Core registration
// receipt. It cannot initialize an absent local claim or dispatch a turn.
func (p *PendingConversations) RecoverChatConversation(ctx context.Context, credential, turn string) (ChatConversationMapping, error) {
	mapping, err := p.LookupChatConversation(ctx, credential, turn)
	if err != nil {
		return mapping, err
	}
	local, err := p.Recover(ctx, credential, personalChatCreationKey(turn))
	if err != nil {
		return mapping, err
	}
	return ChatConversationMapping{ConversationID: local.ID, ClientTurnID: turn, Phase: local.Phase}, nil
}
func (s *AgentChat) RecoverChatConversation(ctx context.Context, credential, turn string) (ChatConversationMapping, error) {
	return s.pending.RecoverChatConversation(ctx, credential, turn)
}
func (s *PersonalChat) RecoverChatConversation(ctx context.Context, credential, turn string) (ChatConversationMapping, error) {
	return s.pending.RecoverChatConversation(ctx, credential, turn)
}
