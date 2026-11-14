// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"database/sql"
	"regexp"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// ProjectLegacyHistory copies only a reviewed frozen-source batch. Live-only
// corpora are never unioned here. Original stage payloads remain owner-only.
func ProjectLegacyHistory(ctx context.Context, tx *sql.Tx, runtimeRole, operation, batch string, recoverOnly bool) error {
	if tx == nil || !regexp.MustCompile(`^[A-Za-z0-9_-]{16,64}$`).MatchString(operation) || batch == "" || len(batch) > 128 {
		return access.ErrInvalid
	}
	if err := historyOwner(ctx, tx, runtimeRole); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `LOCK TABLE lmm_assistant.contract,lmm_assistant.history_projections,lmm_assistant.conversations,lmm_assistant.history_messages IN ACCESS EXCLUSIVE MODE`); err != nil {
		return access.ErrUnavailable
	}
	var version int
	if tx.QueryRowContext(ctx, `SELECT version FROM lmm_assistant.contract WHERE singleton`).Scan(&version) != nil || version != 3 {
		return access.ErrUnavailable
	}
	var same bool
	err := tx.QueryRowContext(ctx, `SELECT operation_id=$2 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND installed_by=session_user FROM lmm_assistant.history_projections WHERE batch=$1`, batch, operation).Scan(&same)
	if err == nil {
		if same {
			return nil
		}
		return access.ErrConflict
	}
	if err != sql.ErrNoRows {
		return access.ErrUnavailable
	}
	if recoverOnly {
		return access.ErrNotFound
	}
	if tx.QueryRowContext(ctx, `SELECT NOT EXISTS(SELECT FROM lmm_assistant.conversations) AND NOT EXISTS(SELECT FROM lmm_assistant.history_messages)`).Scan(&same) != nil || !same {
		return access.ErrConflict
	}
	if _, err = tx.ExecContext(ctx, `LOCK TABLE lmm_assistant.legacy_restore_batches,lmm_assistant.legacy_restore_conversations,lmm_assistant.legacy_restore_messages IN SHARE MODE`); err != nil {
		return access.ErrUnavailable
	}
	if tx.QueryRowContext(ctx, `SELECT NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='lmm_assistant' AND c.relname IN('legacy_restore_batches','legacy_restore_conversations','legacy_restore_messages','legacy_restore_support_requests') AND (has_table_privilege($1,c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') OR has_any_column_privilege($1,c.oid,'SELECT,INSERT,UPDATE,REFERENCES')))`, runtimeRole).Scan(&same) != nil || !same {
		return access.ErrForbidden
	}
	if tx.QueryRowContext(ctx, `SELECT visibility='owner_only_raw' AND lineage->>'source_kind'='FROZEN_OFFLINE_LEGACY' AND (lineage->>'conversations')::bigint=(SELECT count(*) FROM lmm_assistant.legacy_restore_conversations WHERE batch=$1) AND (lineage->>'messages')::bigint=(SELECT count(*) FROM lmm_assistant.legacy_restore_messages WHERE batch=$1) AND (lineage->>'human')::bigint=(SELECT count(*) FROM lmm_assistant.legacy_restore_messages WHERE batch=$1 AND role='human') FROM lmm_assistant.legacy_restore_batches WHERE batch=$1`, batch).Scan(&same) != nil || !same {
		return access.ErrForbidden
	}
	// Check the stage's original ownership and message linkage again inside the
	// same transaction; no ledger-account or timestamp-based owner inference.
	if tx.QueryRowContext(ctx, `SELECT NOT EXISTS(SELECT FROM lmm_assistant.legacy_restore_conversations c WHERE batch=$1 AND (owner_user_id<>(payload->>'user_id')::bigint OR source_id<>(payload->>'id')::bigint)) AND NOT EXISTS(SELECT FROM lmm_assistant.legacy_restore_messages m LEFT JOIN lmm_assistant.legacy_restore_conversations c ON c.batch=m.batch AND c.source_id=m.conversation_id WHERE m.batch=$1 AND (c.source_id IS NULL OR m.source_id<>(m.payload->>'id')::bigint OR m.conversation_id<>(m.payload->>'conversation_id')::bigint OR m.sequence<>(m.payload->>'sequence')::bigint OR m.role<>m.payload->>'role' OR m.actor_user_id IS DISTINCT FROM (m.payload->>'actor_user_id')::bigint))`, batch).Scan(&same) != nil || !same {
		return access.ErrConflict
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.conversations(id,user_id,title,last_message_preview,created_at,updated_at,archived_at,restricted_at,restriction_reason,legacy_list_hidden) OVERRIDING SYSTEM VALUE
 SELECT source_id,owner_user_id,payload->>'title',payload->>'last_message_preview',(payload->>'created_at')::bigint,(payload->>'updated_at')::bigint,coalesce((payload->>'archived_at')::bigint,0),coalesce((payload->>'restricted_at')::bigint,0),coalesce(payload->>'restriction_reason',''),NOT EXISTS(SELECT FROM lmm_assistant.legacy_restore_messages m WHERE m.batch=c.batch AND m.conversation_id=c.source_id)
 FROM lmm_assistant.legacy_restore_conversations c WHERE batch=$1`, batch)
	if err != nil {
		return access.ErrUnavailable
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.history_messages(id,conversation_id,sequence,role,actor_user_id,content,created_at) OVERRIDING SYSTEM VALUE
 SELECT source_id,conversation_id,sequence,role,NULLIF(actor_user_id,0),payload->>'content',(payload->>'created_at')::bigint FROM lmm_assistant.legacy_restore_messages WHERE batch=$1`, batch)
	if err != nil {
		return access.ErrUnavailable
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.history_projections SELECT $1,$2,oid,session_user,(SELECT count(*) FROM lmm_assistant.conversations),(SELECT count(*) FROM lmm_assistant.history_messages),(SELECT count(*) FROM lmm_assistant.history_messages WHERE role='human'),(SELECT count(*) FROM lmm_assistant.conversations WHERE legacy_list_hidden) FROM pg_database WHERE datname=current_database()`, batch, operation)
	if err != nil {
		return access.ErrUnavailable
	}
	for _, table := range []string{"conversations", "history_messages"} {
		if _, err = tx.ExecContext(ctx, `SELECT setval(pg_get_serial_sequence('lmm_assistant.`+table+`','id'),coalesce((SELECT max(id) FROM lmm_assistant.`+table+`),1),EXISTS(SELECT FROM lmm_assistant.`+table+`))`); err != nil {
			return access.ErrUnavailable
		}
	}
	return nil
}
