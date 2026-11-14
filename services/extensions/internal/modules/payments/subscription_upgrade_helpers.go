package payments

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
)

func subscriptionCatalogBySQL(ctx context.Context, tx *sql.Tx, query string, successor bool) (string, error) {
	rows, err := tx.QueryContext(ctx, query, successor)
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

func subscriptionPaymentPredecessorSnapshot(ctx context.Context, tx *sql.Tx) (string, string, error) {
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
