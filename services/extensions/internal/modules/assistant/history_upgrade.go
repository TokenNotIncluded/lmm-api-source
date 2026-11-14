// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"database/sql"
	_ "embed"
	"regexp"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/lib/pq"
)

//go:embed history_upgrade.sql
var historyUpgradeSQL string

// UpgradeHistory is an offline owner operation. The runtime must be fenced;
// ordinary startup never migrates data or changes grants. An unknown COMMIT
// can be checked using the original operation ID, not retried with a new one.
func UpgradeHistory(ctx context.Context, tx *sql.Tx, runtimeRole, operation string, recoverOnly bool) error {
	if tx == nil || runtimeRole == "" || !regexp.MustCompile(`^[A-Za-z0-9_-]{16,64}$`).MatchString(operation) {
		return access.ErrInvalid
	}
	if err := historyOwner(ctx, tx, runtimeRole); err != nil {
		return err
	}
	return upgradeHistory(ctx, tx, runtimeRole, operation, recoverOnly)
}

func historyOwner(ctx context.Context, tx *sql.Tx, runtimeRole string) error {
	var allowed bool
	err := tx.QueryRowContext(ctx, `SELECT current_user=pg_get_userbyid(d.datdba)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname=$1))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_assistant' AND nspowner<>d.datdba)
 FROM pg_database d WHERE datname=current_database()`, runtimeRole).Scan(&allowed)
	if err != nil || !allowed {
		return access.ErrForbidden
	}
	return nil
}

func upgradeHistory(ctx context.Context, tx *sql.Tx, runtimeRole, operation string, recoverOnly bool) error {
	var err error
	if _, err = tx.ExecContext(ctx, `LOCK TABLE lmm_assistant.contract,lmm_assistant.conversations,lmm_assistant.history_messages IN ACCESS EXCLUSIVE MODE`); err != nil {
		return access.ErrUnavailable
	}
	var version int
	if err = tx.QueryRowContext(ctx, `SELECT version FROM lmm_assistant.contract WHERE singleton`).Scan(&version); err != nil {
		return access.ErrUnavailable
	}
	if version == 3 {
		var same bool
		if tx.QueryRowContext(ctx, `SELECT operation_id=$1 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND installed_by=session_user FROM lmm_assistant.history_upgrades WHERE version=3`, operation).Scan(&same) != nil || !same {
			return access.ErrConflict
		}
		return nil
	}
	if version != 2 {
		return access.ErrUnavailable
	}
	if recoverOnly {
		return access.ErrNotFound
	}
	if _, err = tx.ExecContext(ctx, historyUpgradeSQL); err != nil {
		return access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `REVOKE ALL ON lmm_assistant.history_upgrades,lmm_assistant.history_projections FROM `+pq.QuoteIdentifier(runtimeRole)); err != nil {
		return access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.history_upgrades SELECT 3,$1,oid,session_user FROM pg_database WHERE datname=current_database()`, operation); err != nil {
		return access.ErrUnavailable
	}
	return nil
}
