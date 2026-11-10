// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"database/sql"
	"strconv"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// AgentUnstartedFence never treats an unlocked absent observation as proof that
// a request stopped. It closes the original conversation under the same locks
// used to commit a start intent. Existing intents must recover/cancel in Core.
type AgentUnstartedFence interface {
	FenceUnstarted(context.Context, int64, int64, string) (bool, error)
}

func (p *AgentPostgresJournal) FenceUnstarted(ctx context.Context, owner, conversation int64, turn string) (bool, error) {
	if owner <= 0 || !validAgentChatID(conversation, turn) {
		return false, access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return false, access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "lmm_assistant_delegation:"+strconv.FormatInt(conversation, 10)); err != nil {
		return false, access.ErrUnavailable
	}
	var restricted int64
	if err = tx.QueryRowContext(ctx, `SELECT restricted_at FROM lmm_assistant.conversations WHERE id=$1 AND user_id=$2 FOR UPDATE`, conversation, owner).Scan(&restricted); err != nil {
		return false, historyError(err)
	}
	var key string
	if err = tx.QueryRowContext(ctx, `SELECT creation_key FROM lmm_assistant.delegation_conversations WHERE user_id=$1 AND conversation_id=$2`, owner, conversation).Scan(&key); err != nil {
		return false, historyError(err)
	}
	if key != personalChatCreationKey(turn) {
		return false, access.ErrNotFound
	}
	var exists bool
	if err = tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM lmm_assistant.agent_turns WHERE user_id=$1 AND turn_id=$2)`, owner, turn).Scan(&exists); err != nil {
		return false, access.ErrUnavailable
	}
	if exists {
		return false, nil
	}
	// This flag is durable and is not cleared by ordinary unarchive. Every later
	// WithinAgent observes CanStart=false before it can create an original intent.
	if restricted == 0 {
		if _, err = tx.ExecContext(ctx, `UPDATE lmm_assistant.conversations SET restricted_at=$3 WHERE id=$1 AND user_id=$2`, conversation, owner, time.Now().Unix()); err != nil {
			return false, access.ErrUnavailable
		}
	}
	if tx.Commit() != nil {
		return false, ErrDelegationUnknown
	}
	return true, nil
}
