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

//go:embed subscription_pancake_catalog.sql
var pancakeSubscriptionCatalogSQL string

//go:embed subscription_pancake_storage_contract.json
var pancakeSubscriptionContractJSON []byte

const SubscriptionPancakeUpgradeContract = "lmm-payments-subscription-pancake-v1"

type SubscriptionPancakeUpgradeReceipt SubscriptionEPayUpgradeReceipt

func pancakeCatalog(ctx context.Context, tx *sql.Tx, successor bool) (string, error) {
	rows, err := tx.QueryContext(ctx, pancakeSubscriptionCatalogSQL, successor)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	h := sha256.New()
	for rows.Next() {
		var fact string
		if err = rows.Scan(&fact); err != nil {
			return "", err
		}
		frame(h, fact)
	}
	return hex.EncodeToString(h.Sum(nil)), rows.Err()
}
func pancakePredecessorSnapshot(ctx context.Context, tx *sql.Tx) (string, string, error) {
	data, acl := sha256.New(), sha256.New()
	for _, table := range []string{"payment_storage_guard", "payment_orders", "payment_events", "payment_subscription_inbox", "payment_subscription_upgrades"} {
		var exists bool
		if err := tx.QueryRowContext(ctx, `SELECT to_regclass('public.'||$1) IS NOT NULL`, table).Scan(&exists); err != nil {
			return "", "", err
		}
		if !exists {
			continue
		}
		frame(data, table)
		rows, err := tx.QueryContext(ctx, `SELECT encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex'),count(*)::text FROM public.`+table+` t GROUP BY 1 ORDER BY 1`)
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
	rows, err := tx.QueryContext(ctx, `SELECT fact FROM (
 SELECT json_build_array('schema',n.nspowner,n.nspacl)::text AS fact FROM pg_namespace n WHERE n.nspname='public'
 UNION ALL SELECT json_build_array('relation',c.relname,c.relowner,c.relacl)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname IN ('payment_storage_guard','payment_orders','payment_events','payment_subscription_inbox','payment_subscription_upgrades')
 UNION ALL SELECT json_build_array('column',c.relname,a.attnum,a.attacl)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid WHERE n.nspname='public' AND c.relname IN ('payment_storage_guard','payment_orders','payment_events','payment_subscription_inbox','payment_subscription_upgrades') AND a.attnum>0 AND NOT a.attisdropped
 UNION ALL SELECT json_build_array('function',p.proname,p.proowner,p.proacl)::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname='guard_payment_subscription_evidence'
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
func UpgradeSubscriptionPancake(ctx context.Context, db *sql.DB, expectedDatabase, runtimeRole string) (SubscriptionPancakeUpgradeReceipt, error) {
	var receipt SubscriptionPancakeUpgradeReceipt
	if db == nil || expectedDatabase == "" || !paymentRuntimeRole.MatchString(runtimeRole) {
		return receipt, ErrInvalid
	}
	var target struct {
		Schema string `json:"pancake_schema_sha256"`
		Before string `json:"predecessor_catalog_sha256"`
		After  string `json:"successor_catalog_sha256"`
	}
	if json.Unmarshal(pancakeSubscriptionContractJSON, &target) != nil || target.Schema != fmt.Sprintf("%x", sha256.Sum256([]byte(SubscriptionPancakeSchema))) {
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
	for _, q := range []string{"SET LOCAL search_path=pg_catalog", "SET LOCAL lock_timeout='2s'", "SET LOCAL statement_timeout='10s'", "SET LOCAL synchronous_commit=on", "SELECT pg_advisory_xact_lock(hashtextextended('lmm-payments-subscription-pancake-upgrade',0))"} {
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
	current, err := pancakeCatalog(ctx, tx, true)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if current == target.After {
		var raw []byte
		if err = tx.QueryRowContext(ctx, `SELECT receipt FROM public.payment_pancake_subscription_upgrades WHERE contract=$1`, SubscriptionPancakeUpgradeContract).Scan(&raw); err != nil {
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
	before, acl, err := pancakePredecessorSnapshot(ctx, tx)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=public,pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, SubscriptionPancakeSchema); err != nil {
		return receipt, ErrUnavailable
	}
	role := `"` + strings.ReplaceAll(runtimeRole, `"`, `""`) + `"`
	if _, err = tx.ExecContext(ctx, `GRANT SELECT,INSERT,UPDATE ON public.payment_pancake_subscription_checkouts,public.payment_pancake_subscription_inbox,public.payment_pancake_wallet_inbox TO `+role); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	after, afterACL, err := pancakePredecessorSnapshot(ctx, tx)
	if err != nil || after != before || afterACL != acl {
		return receipt, ErrConflict
	}
	current, err = pancakeCatalog(ctx, tx, true)
	if err != nil || current != target.After {
		return receipt, ErrConflict
	}
	receipt = SubscriptionPancakeUpgradeReceipt{Contract: SubscriptionPancakeUpgradeContract, Database: database, RuntimeRole: runtimeRole, Status: "committed", PredecessorCatalogSHA256: target.Before, SuccessorCatalogSHA256: target.After, PreservedDataSHA256: before, PreservedACLSHA256: acl}
	raw, err := json.Marshal(receipt)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `CREATE TABLE public.payment_pancake_subscription_upgrades(contract TEXT PRIMARY KEY,receipt BYTEA NOT NULL,committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()); REVOKE ALL ON public.payment_pancake_subscription_upgrades FROM PUBLIC`); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO public.payment_pancake_subscription_upgrades(contract,receipt) VALUES($1,$2)`, SubscriptionPancakeUpgradeContract, raw); err != nil {
		return receipt, ErrUnavailable
	}
	if err = tx.Commit(); err != nil {
		return receipt, ErrUnavailable
	}
	return receipt, nil
}
