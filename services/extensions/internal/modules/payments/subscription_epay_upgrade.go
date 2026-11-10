package payments

import (
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"hash"
	"regexp"
	"strings"
	"time"
)

//go:embed subscription_epay_catalog.sql
var subscriptionCatalogSQL string

//go:embed subscription_epay_storage_contract.json
var subscriptionStorageContractJSON []byte

const SubscriptionEPayUpgradeContract = "lmm-payments-subscription-epay-v1"

type subscriptionStorageContract struct {
	BaseSchemaSHA256         string `json:"base_schema_sha256"`
	SubscriptionSchemaSHA256 string `json:"subscription_schema_sha256"`
	PredecessorCatalogSHA256 string `json:"predecessor_catalog_sha256"`
	SuccessorCatalogSHA256   string `json:"successor_catalog_sha256"`
}
type SubscriptionEPayUpgradeReceipt struct {
	Contract                 string `json:"contract"`
	Database                 string `json:"database"`
	RuntimeRole              string `json:"runtime_role"`
	Status                   string `json:"status"`
	PredecessorCatalogSHA256 string `json:"predecessor_catalog_sha256"`
	SuccessorCatalogSHA256   string `json:"successor_catalog_sha256"`
	PreservedDataSHA256      string `json:"preserved_data_sha256"`
	PreservedACLSHA256       string `json:"preserved_acl_sha256"`
}

func subscriptionContract() (subscriptionStorageContract, error) {
	var c subscriptionStorageContract
	if json.Unmarshal(subscriptionStorageContractJSON, &c) != nil || fmt.Sprintf("%x", sha256.Sum256([]byte(Schema))) != c.BaseSchemaSHA256 || fmt.Sprintf("%x", sha256.Sum256([]byte(SubscriptionEPaySchema))) != c.SubscriptionSchemaSHA256 {
		return c, ErrConflict
	}
	return c, nil
}
func frame(h hash.Hash, s string) {
	var b [8]byte
	binary.BigEndian.PutUint64(b[:], uint64(len(s)))
	_, _ = h.Write(b[:])
	_, _ = h.Write([]byte(s))
}
func subscriptionCatalog(ctx context.Context, tx *sql.Tx, inbox bool) (string, error) {
	rows, err := tx.QueryContext(ctx, subscriptionCatalogSQL, inbox)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	h := sha256.New()
	for rows.Next() {
		var s string
		if err = rows.Scan(&s); err != nil {
			return "", err
		}
		frame(h, s)
	}
	return hex.EncodeToString(h.Sum(nil)), rows.Err()
}
func paymentRows(ctx context.Context, tx *sql.Tx) (string, error) {
	h := sha256.New()
	for _, table := range []string{"payment_storage_guard", "payment_orders", "payment_events"} {
		frame(h, table)
		rows, err := tx.QueryContext(ctx, `SELECT encode(sha256(convert_to(to_jsonb(t)::text,'UTF8')),'hex'),count(*)::text FROM public.`+table+` t GROUP BY 1 ORDER BY 1`)
		if err != nil {
			return "", err
		}
		for rows.Next() {
			var digest, count string
			if err = rows.Scan(&digest, &count); err != nil {
				_ = rows.Close()
				return "", err
			}
			frame(h, digest)
			frame(h, count)
		}
		err = rows.Err()
		_ = rows.Close()
		if err != nil {
			return "", err
		}
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}
func paymentACL(ctx context.Context, tx *sql.Tx) (string, error) {
	rows, err := tx.QueryContext(ctx, `SELECT fact FROM (
 SELECT json_build_array('schema',n.nspowner,n.nspacl)::text fact FROM pg_namespace n WHERE n.nspname='public'
 UNION ALL SELECT json_build_array('relation',c.relname,c.relowner,c.relacl)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname IN ('payment_storage_guard','payment_orders','payment_events')
 UNION ALL SELECT json_build_array('column',c.relname,a.attnum,a.attacl)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid WHERE n.nspname='public' AND c.relname IN ('payment_storage_guard','payment_orders','payment_events') AND a.attnum>0 AND NOT a.attisdropped
 ) q ORDER BY fact COLLATE "C"`)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	h := sha256.New()
	for rows.Next() {
		var s string
		if err = rows.Scan(&s); err != nil {
			return "", err
		}
		frame(h, s)
	}
	return hex.EncodeToString(h.Sum(nil)), rows.Err()
}

var paymentRuntimeRole = regexp.MustCompile(`^[a-z_][a-z0-9_]{0,62}$`)

// UpgradeSubscriptionEPay is an explicit OFFLINE owner operation. It is never
// called by NewPostgres, host startup or workers. Drain the payment host first.
// Exact Core-free database, PG18 catalog and original rows/ACLs are required.
func UpgradeSubscriptionEPay(ctx context.Context, db *sql.DB, expectedDatabase, runtimeRole string) (SubscriptionEPayUpgradeReceipt, error) {
	var receipt SubscriptionEPayUpgradeReceipt
	if db == nil || expectedDatabase == "" || !paymentRuntimeRole.MatchString(runtimeRole) {
		return receipt, ErrInvalid
	}
	c, err := subscriptionContract()
	if err != nil {
		return receipt, err
	}
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	tx, err := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return receipt, ErrUnavailable
	}
	defer tx.Rollback()
	for _, statement := range []string{"SET LOCAL search_path=pg_catalog", "SET LOCAL lock_timeout='2s'", "SET LOCAL statement_timeout='10s'", "SET LOCAL synchronous_commit=on", "SELECT pg_advisory_xact_lock(hashtextextended('lmm-payments-subscription-epay-upgrade',0))"} {
		if _, err = tx.ExecContext(ctx, statement); err != nil {
			return receipt, ErrUnavailable
		}
	}
	var database string
	var safe bool
	err = tx.QueryRowContext(ctx, `SELECT current_database(),current_setting('server_version_num')::int BETWEEN 180000 AND 189999
 AND current_user=pg_get_userbyid(d.datdba) AND current_user<>$1
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles outsider JOIN pg_roles target ON target.rolname=$1 WHERE outsider.rolcanlogin AND outsider.rolname<>session_user AND NOT outsider.rolsuper AND pg_has_role(outsider.oid,target.oid,'SET'))
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid())
 FROM pg_database d WHERE d.datname=current_database()`, runtimeRole).Scan(&database, &safe)
	if err != nil || !safe || database != expectedDatabase {
		return receipt, ErrDenied
	}
	if _, err = tx.ExecContext(ctx, `LOCK TABLE public.payment_storage_guard,public.payment_orders,public.payment_events IN ACCESS EXCLUSIVE MODE`); err != nil {
		return receipt, ErrUnavailable
	}
	var contract string
	if err = tx.QueryRowContext(ctx, `SELECT contract FROM public.payment_storage_guard WHERE singleton`).Scan(&contract); err != nil || contract != "lmm-payments-v1" {
		return receipt, ErrConflict
	}
	catalog, err := subscriptionCatalog(ctx, tx, true)
	if err != nil {
		return receipt, ErrUnavailable
	}
	if catalog == c.SuccessorCatalogSHA256 {
		var raw []byte
		if err = tx.QueryRowContext(ctx, `SELECT receipt FROM public.payment_subscription_upgrades WHERE contract=$1`, SubscriptionEPayUpgradeContract).Scan(&raw); err != nil {
			return receipt, ErrConflict
		}
		if json.Unmarshal(raw, &receipt) != nil || receipt.Database != database || receipt.RuntimeRole != runtimeRole || receipt.SuccessorCatalogSHA256 != catalog {
			return receipt, ErrConflict
		}
		receipt.Status = "already_committed"
		return receipt, nil
	}
	if catalog != c.PredecessorCatalogSHA256 {
		return receipt, ErrConflict
	}
	before, err := paymentRows(ctx, tx)
	if err != nil {
		return receipt, ErrUnavailable
	}
	acl, err := paymentACL(ctx, tx)
	if err != nil {
		return receipt, ErrUnavailable
	}
	// Compiled SQL uses public by default; the offline transaction fixes its path.
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=public,pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, SubscriptionEPaySchema); err != nil {
		return receipt, ErrUnavailable
	}
	role := `"` + strings.ReplaceAll(runtimeRole, `"`, `""`) + `"`
	if _, err = tx.ExecContext(ctx, `GRANT SELECT,INSERT,UPDATE ON public.payment_subscription_inbox TO `+role); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, "SET LOCAL search_path=pg_catalog"); err != nil {
		return receipt, ErrUnavailable
	}
	after, err := paymentRows(ctx, tx)
	if err != nil {
		return receipt, ErrUnavailable
	}
	afterACL, err := paymentACL(ctx, tx)
	if err != nil {
		return receipt, ErrUnavailable
	}
	target, err := subscriptionCatalog(ctx, tx, true)
	if err != nil || after != before || afterACL != acl || target != c.SuccessorCatalogSHA256 {
		return receipt, ErrConflict
	}
	receipt = SubscriptionEPayUpgradeReceipt{Contract: SubscriptionEPayUpgradeContract, Database: database, RuntimeRole: runtimeRole, Status: "committed", PredecessorCatalogSHA256: catalog, SuccessorCatalogSHA256: target, PreservedDataSHA256: before, PreservedACLSHA256: acl}
	raw, _ := json.Marshal(receipt)
	if _, err = tx.ExecContext(ctx, `CREATE TABLE public.payment_subscription_upgrades(contract TEXT PRIMARY KEY CHECK(contract='lmm-payments-subscription-epay-v1'),receipt JSONB NOT NULL,recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()); REVOKE ALL ON public.payment_subscription_upgrades FROM PUBLIC`); err != nil {
		return receipt, ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO public.payment_subscription_upgrades(contract,receipt) VALUES($1,$2)`, SubscriptionEPayUpgradeContract, string(raw)); err != nil {
		return receipt, ErrUnavailable
	}
	if err = tx.Commit(); err != nil {
		return receipt, ErrUnavailable
	}
	return receipt, nil
}
