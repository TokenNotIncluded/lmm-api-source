WITH expected(name,can_delete,mutable) AS (
    VALUES ('affinity_cache',true,ARRAY['rule_name','channel_id','binding_sha256','expires_at','touched_at']),
           ('route_health',false,ARRAY['disabled_scope','reason','status_code','last_probe_at','latency_ms','next_probe_at','lease_until','generation'])
), relations AS (
    SELECT c.oid,c.relkind,c.relacl,e.can_delete,e.mutable
    FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    JOIN expected e ON c.relname=e.name WHERE n.nspname='core_channels'
), cols AS (
    SELECT a.attrelid,a.attnum,a.attname,a.attacl,r.mutable FROM relations r
    JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid WHERE a.attnum>0 AND NOT a.attisdropped
), receipt AS (
    SELECT c.oid,c.relowner,c.relacl,c.relkind FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    WHERE n.nspname='core_meta' AND c.relname='route_runtime_upgrades'
)
SELECT (
    EXISTS (SELECT FROM relations r WHERE r.relkind<>'r'
        OR pg_catalog.has_table_privilege(r.oid,'UPDATE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR pg_catalog.has_table_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,DELETE WITH GRANT OPTION')
        OR (NOT r.can_delete AND pg_catalog.has_table_privilege(r.oid,'DELETE'))
        OR pg_catalog.has_any_column_privilege(r.oid,'REFERENCES,SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
        OR EXISTS (SELECT FROM pg_catalog.aclexplode(r.relacl) a WHERE a.grantee=0))
    OR EXISTS (SELECT FROM cols a WHERE
        (a.attname::text<>ALL(a.mutable) AND pg_catalog.has_column_privilege(a.attrelid,a.attnum,'UPDATE'))
        OR EXISTS (SELECT FROM pg_catalog.aclexplode(a.attacl) p WHERE p.grantee=0))
    OR EXISTS (SELECT FROM receipt r WHERE r.relkind<>'r'
        OR pg_catalog.has_table_privilege(r.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR pg_catalog.has_any_column_privilege(r.oid,'SELECT,INSERT,UPDATE,REFERENCES')
        OR EXISTS (SELECT FROM pg_catalog.aclexplode(r.relacl) p WHERE p.grantee<>r.relowner)
        OR EXISTS (SELECT FROM pg_catalog.pg_attribute a CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) p
            WHERE a.attrelid=r.oid AND p.grantee<>r.relowner))
), (
    (SELECT count(*)=2 FROM relations WHERE relkind='r') AND EXISTS (SELECT FROM receipt WHERE relkind='r')
    AND EXISTS (SELECT FROM pg_catalog.pg_namespace n WHERE n.nspname='core_channels'
        AND pg_catalog.has_schema_privilege(n.oid,'USAGE'))
    AND NOT EXISTS (SELECT FROM relations r WHERE
        NOT pg_catalog.has_table_privilege(r.oid,'SELECT') OR NOT pg_catalog.has_table_privilege(r.oid,'INSERT')
        OR (r.can_delete AND NOT pg_catalog.has_table_privilege(r.oid,'DELETE')))
    AND NOT EXISTS (SELECT FROM relations r CROSS JOIN LATERAL unnest(r.mutable) AS m(name)
        WHERE NOT EXISTS (SELECT FROM cols a WHERE a.attrelid=r.oid AND a.attname=m.name
            AND pg_catalog.has_column_privilege(a.attrelid,a.attnum,'UPDATE')))
)
