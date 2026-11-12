package checkins

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
)

// PublishPolicy is an offline owner operation. Runtime receives SELECT only on
// policy tables and can neither enable rewards nor alter published policy.
func PublishPolicy(ctx context.Context, db *sql.DB, key string, expected int64, p Policy) (int64, error) {
	if !validKey(key) || expected < 0 || expected == 9223372036854775807 || p.Validate() != nil {
		return 0, ErrInvalid
	}
	tx, e := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if e != nil {
		return 0, ErrUnavailable
	}
	defer tx.Rollback()
	if _, e = tx.ExecContext(ctx, "SET LOCAL synchronous_commit=on; SELECT pg_advisory_xact_lock(hashtextextended('lmm-checkins-owner',0))"); e != nil {
		return 0, ErrUnavailable
	}
	raw, _ := json.Marshal(p)
	var revision int64
	var matches bool
	e = tx.QueryRowContext(ctx, "SELECT revision,policy=$2::jsonb FROM lmm_checkins.policy_revisions WHERE operation_key=$1", key, string(raw)).Scan(&revision, &matches)
	if e == nil {
		if !matches || revision != expected+1 {
			return 0, ErrConflict
		}
		if tx.Commit() != nil {
			return 0, ErrUnavailable
		}
		return revision, nil
	}
	if !errors.Is(e, sql.ErrNoRows) {
		return 0, ErrUnavailable
	}
	var current int64
	e = tx.QueryRowContext(ctx, "SELECT revision FROM lmm_checkins.active_policy WHERE singleton").Scan(&current)
	if e != nil && !errors.Is(e, sql.ErrNoRows) {
		return 0, ErrUnavailable
	}
	if current != expected {
		return 0, ErrConflict
	}
	revision = current + 1
	if _, e = tx.ExecContext(ctx, "INSERT INTO lmm_checkins.policy_revisions(revision,policy,operation_key,created_at) VALUES($1,$2::jsonb,$3,floor(extract(epoch FROM transaction_timestamp()))::bigint)", revision, string(raw), key); e != nil {
		return 0, ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, "INSERT INTO lmm_checkins.active_policy(singleton,revision) VALUES(true,$1) ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision", revision); e != nil {
		return 0, ErrUnavailable
	}
	if tx.Commit() != nil {
		return 0, ErrUnavailable
	}
	return revision, nil
}
