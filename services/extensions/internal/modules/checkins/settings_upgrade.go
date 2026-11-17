// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package checkins

import (
	"context"
	"database/sql"
	"regexp"

	"github.com/lib/pq"
)

var settingsUpgradeKey = regexp.MustCompile(`^[A-Za-z0-9_-]{16,64}$`)

// UpgradeSettings is an offline, fenced owner operation. The original policy,
// rewards and receipts remain intact. Runtime gains only the exact function.
func UpgradeSettings(ctx context.Context, tx *sql.Tx, runtime, operation string, recoverOnly bool) error {
	if runtime == "" || !settingsUpgradeKey.MatchString(operation) {
		return ErrInvalid
	}
	var allowed bool
	e := tx.QueryRowContext(ctx, `SELECT current_user=pg_get_userbyid(d.datdba)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin
   AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_checkins' AND nspowner<>d.datdba)
 FROM pg_database d WHERE datname=current_database()`, runtime).Scan(&allowed)
	if e != nil || !allowed {
		return ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, `LOCK TABLE lmm_checkins.contract,lmm_checkins.policy_revisions,lmm_checkins.active_policy IN ACCESS EXCLUSIVE MODE`); e != nil {
		return ErrUnavailable
	}
	var version int
	if tx.QueryRowContext(ctx, "SELECT version FROM lmm_checkins.contract WHERE singleton").Scan(&version) != nil || version != 1 {
		return ErrUnavailable
	}
	var installed, receipt bool
	if tx.QueryRowContext(ctx, `SELECT to_regprocedure($1) IS NOT NULL,to_regclass('lmm_checkins.settings_upgrade') IS NOT NULL`, settingsFunction).Scan(&installed, &receipt) != nil {
		return ErrUnavailable
	}
	if installed {
		if !receipt {
			return ErrConflict
		}
		var same bool
		e = tx.QueryRowContext(ctx, `SELECT operation_id=$1 AND installed_by=session_user AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) FROM lmm_checkins.settings_upgrade WHERE singleton`, operation).Scan(&same)
		if e != nil || !same {
			return ErrConflict
		}
		return nil
	}
	if receipt || recoverOnly {
		return ErrConflict
	}
	if _, e = tx.ExecContext(ctx, SettingsSchema); e != nil {
		return ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, `CREATE TABLE lmm_checkins.settings_upgrade(
 singleton boolean PRIMARY KEY CHECK(singleton),operation_id text NOT NULL,
 installed_by name NOT NULL,database_oid oid NOT NULL);
 REVOKE ALL ON lmm_checkins.settings_upgrade FROM PUBLIC;`); e != nil {
		return ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, "GRANT EXECUTE ON FUNCTION "+settingsFunction+" TO "+pq.QuoteIdentifier(runtime)); e != nil {
		return ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, `INSERT INTO lmm_checkins.settings_upgrade SELECT true,$1,session_user,oid FROM pg_database WHERE datname=current_database()`, operation); e != nil {
		return ErrUnavailable
	}
	return nil
}
