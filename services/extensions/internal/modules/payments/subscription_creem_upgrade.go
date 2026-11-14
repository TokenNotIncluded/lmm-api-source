package payments

import (
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

//go:embed subscription_creem_catalog.sql
var creemSubscriptionCatalogSQL string

//go:embed subscription_creem_storage_contract.json
var creemSubscriptionContractJSON []byte

const SubscriptionCreemUpgradeContract = "lmm-payments-subscription-creem-v1"

type SubscriptionCreemUpgradeReceipt SubscriptionEPayUpgradeReceipt

func creemCatalog(ctx context.Context, tx *sql.Tx, successor bool) (string, error) {
	return subscriptionCatalogBySQL(ctx, tx, creemSubscriptionCatalogSQL, successor)
}
func creemPredecessorSnapshot(ctx context.Context, tx *sql.Tx) (string, string, error) {
	data, acl := sha256.New(), sha256.New()
	rows, err := tx.QueryContext(ctx, `SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='r' AND starts_with(c.relname,'payment_') AND NOT starts_with(c.relname,'payment_creem_') ORDER BY c.relname`)
	if err != nil {
		return "", "", err
	}
	var tables []string
	for rows.Next() {
		var table string
		if err = rows.Scan(&table); err != nil {
			rows.Close()
			return "", "", err
		}
		tables = append(tables, table)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return "", "", err
	}
	for _, table := range tables {
		frame(data, table)
		quoted := `"` + strings.ReplaceAll(table, `"`, `""`) + `"`
		rows, err = tx.QueryContext(ctx, `SELECT encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex'),count(*)::text FROM public.`+quoted+` t GROUP BY 1 ORDER BY 1`)
		if err != nil {
			return "", "", err
		}
		for rows.Next() {
			var hash, count string
			if err = rows.Scan(&hash, &count); err != nil {
				rows.Close()
				return "", "", err
			}
			frame(data, hash)
			frame(data, count)
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return "", "", err
		}
	}
	rows, err = tx.QueryContext(ctx, `SELECT fact FROM (
 SELECT json_build_array('schema',n.nspowner,n.nspacl)::text fact FROM pg_namespace n WHERE n.nspname='public'
 UNION ALL SELECT json_build_array('relation',c.relname,c.relowner,c.relacl)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND starts_with(c.relname,'payment_') AND NOT starts_with(c.relname,'payment_creem_')
 UNION ALL SELECT json_build_array('column',c.relname,a.attnum,a.attacl)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid WHERE n.nspname='public' AND starts_with(c.relname,'payment_') AND NOT starts_with(c.relname,'payment_creem_') AND a.attnum>0 AND NOT a.attisdropped
 UNION ALL SELECT json_build_array('function',p.proname,p.proowner,p.proacl,pg_get_functiondef(p.oid))::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND starts_with(p.proname,'guard_') AND p.proname<>'guard_creem_subscription_recovery'
 ) q ORDER BY fact COLLATE "C"`)
	if err != nil {
		return "", "", err
	}
	defer rows.Close()
	for rows.Next() {
		var fact string
		if err = rows.Scan(&fact); err != nil {
			return "", "", err
		}
		frame(acl, fact)
	}
	return hex.EncodeToString(data.Sum(nil)), hex.EncodeToString(acl.Sum(nil)), rows.Err()
}

// Explicit fenced OFFLINE installation only. No startup auto-DDL. The checked-in
// successor pin MUST be captured by real PG18 before this can mutate anything.
func UpgradeSubscriptionCreem(ctx context.Context, db *sql.DB, expectedDatabase, runtimeRole string) (SubscriptionCreemUpgradeReceipt, error) {
	var receipt SubscriptionCreemUpgradeReceipt
	if db == nil || expectedDatabase == "" || !paymentRuntimeRole.MatchString(runtimeRole) {
		return receipt, ErrInvalid
	}
	var target struct {
		Schema string `json:"creem_schema_sha256"`
		Before string `json:"predecessor_catalog_sha256"`
		After  string `json:"successor_catalog_sha256"`
	}
	if json.Unmarshal(creemSubscriptionContractJSON, &target) != nil || target.Schema != fmt.Sprintf("%x", sha256.Sum256([]byte(SubscriptionCreemSchema))) {
		return receipt, ErrConflict
	}
	if len(target.After) != 64 {
		return receipt, ErrUnavailable
	} // no invented catalog pin
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	tx, err := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return receipt, ErrUnavailable
	}
	defer tx.Rollback()
	for _, q := range []string{"SET LOCAL search_path=pg_catalog", "SET LOCAL lock_timeout='2s'", "SET LOCAL statement_timeout='10s'", "SET LOCAL synchronous_commit=on", "SELECT pg_advisory_xact_lock(hashtextextended('lmm-payments-subscription-creem-upgrade',0))"} {
		if _, err = tx.ExecContext(ctx, q); err != nil {
			return receipt, ErrUnavailable
		}
	}
	var database string
	var safe bool
	err = tx.QueryRowContext(ctx, `SELECT current_database(),current_setting('server_version_num')::int BETWEEN 180000 AND 189999
 AND current_user=pg_get_userbyid(d.datdba) AND current_user<>$1
 AND (SELECT count(*)=2 FROM pg_roles WHERE rolname IN(current_user,$1) AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(current_user,session_user,$1) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles outsider JOIN pg_roles target ON target.rolname=$1 WHERE outsider.rolcanlogin AND outsider.rolname<>session_user AND NOT outsider.rolsuper AND (has_database_privilege(outsider.oid,d.oid,'CONNECT') OR pg_has_role(outsider.oid,current_user,'SET') OR pg_has_role(outsider.oid,target.oid,'SET')))
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid())
 FROM pg_database d WHERE d.datname=current_database()`, runtimeRole).Scan(&database, &safe)
	if err != nil || !safe || database != expectedDatabase {
		return receipt, ErrDenied
	}
	if _, err = tx.ExecContext(ctx, `LOCK TABLE public.payment_storage_guard,public.payment_orders,public.payment_events,public.payment_subscription_inbox IN ACCESS EXCLUSIVE MODE`); err != nil {
		return receipt, ErrUnavailable
	}
	current, err := creemCatalog(ctx, tx, true)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if current == target.After {
		var raw []byte
		if err = tx.QueryRowContext(ctx, `SELECT receipt FROM public.payment_creem_subscription_upgrades WHERE contract=$1`, SubscriptionCreemUpgradeContract).Scan(&raw); err != nil {
			return receipt, ErrConflict
		}
		if json.Unmarshal(raw, &receipt) != nil || receipt.Database != database || receipt.RuntimeRole != runtimeRole || receipt.SuccessorCatalogSHA256 != current {
			return receipt, ErrConflict
		}
		receipt.Status = "already_committed"
		return receipt, nil
	}
	if current != target.Before {
		return receipt, ErrConflict
	}
	before, acl, err := creemPredecessorSnapshot(ctx, tx)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=public,pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, SubscriptionCreemSchema); err != nil {
		return receipt, ErrUnavailable
	}
	role := `"` + strings.ReplaceAll(runtimeRole, `"`, `""`) + `"`
	if _, err = tx.ExecContext(ctx, `GRANT SELECT,INSERT,UPDATE ON public.payment_creem_subscription_checkouts,public.payment_creem_subscription_inbox TO `+role); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	// pg_get_constraintdef qualifies references according to search_path.
	// Use the same catalog-only view for capture, verification and receipt replay.
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	after, afterACL, err := creemPredecessorSnapshot(ctx, tx)
	if err != nil || after != before || afterACL != acl {
		return receipt, ErrConflict
	}
	current, err = creemCatalog(ctx, tx, true)
	if err != nil || current != target.After {
		return receipt, ErrConflict
	}
	receipt = SubscriptionCreemUpgradeReceipt{Contract: SubscriptionCreemUpgradeContract, Database: database, RuntimeRole: runtimeRole, Status: "committed", PredecessorCatalogSHA256: target.Before, SuccessorCatalogSHA256: target.After, PreservedDataSHA256: before, PreservedACLSHA256: acl}
	raw, err := json.Marshal(receipt)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `CREATE TABLE public.payment_creem_subscription_upgrades(contract TEXT PRIMARY KEY,receipt BYTEA NOT NULL,committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()); REVOKE ALL ON public.payment_creem_subscription_upgrades FROM PUBLIC`); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO public.payment_creem_subscription_upgrades(contract,receipt) VALUES($1,$2)`, SubscriptionCreemUpgradeContract, raw); err != nil {
		return receipt, ErrUnavailable
	}
	if err = tx.Commit(); err != nil {
		return receipt, ErrUnavailable
	}
	return receipt, nil
}
