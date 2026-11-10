WITH rels AS (
 SELECT c.* FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='public' AND c.relname IN ('payment_storage_guard','payment_orders','payment_events','payment_subscription_inbox','payment_pancake_subscription_checkouts','payment_pancake_subscription_inbox','payment_pancake_wallet_inbox')
 AND ($1::boolean OR c.relname NOT IN ('payment_pancake_subscription_checkouts','payment_pancake_subscription_inbox','payment_pancake_wallet_inbox'))
), facts AS (
 SELECT json_build_array('relation',r.relname,r.relkind,r.relrowsecurity,r.relforcerowsecurity)::text AS fact FROM rels r
 UNION ALL SELECT json_build_array('column',r.relname,a.attnum,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,a.attidentity,a.attgenerated,pg_get_expr(d.adbin,d.adrelid))::text
 FROM rels r JOIN pg_attribute a ON a.attrelid=r.oid LEFT JOIN pg_attrdef d ON d.adrelid=r.oid AND d.adnum=a.attnum WHERE a.attnum>0 AND NOT a.attisdropped
 UNION ALL SELECT json_build_array('constraint',r.relname,c.conname,c.contype,pg_get_constraintdef(c.oid))::text FROM rels r JOIN pg_constraint c ON c.conrelid=r.oid
 UNION ALL SELECT json_build_array('index',r.relname,pg_get_indexdef(i.indexrelid))::text FROM rels r JOIN pg_index i ON i.indrelid=r.oid
 UNION ALL SELECT json_build_array('trigger',r.relname,t.tgname,pg_get_triggerdef(t.oid),t.tgenabled)::text FROM rels r JOIN pg_trigger t ON t.tgrelid=r.oid WHERE NOT t.tgisinternal
 UNION ALL SELECT json_build_array('function',p.proname,pg_get_functiondef(p.oid))::text
 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='public' AND (p.proname='guard_payment_subscription_evidence' OR ($1::boolean AND p.proname='guard_pancake_subscription_recovery'))
) SELECT fact FROM facts ORDER BY fact COLLATE "C"
