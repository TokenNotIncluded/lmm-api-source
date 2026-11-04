//! Source-scoped historical observations. Runtime never restores old authority.
pub(super) const SQL: &str = r#"WITH users AS MATERIALIZED (
 SELECT u.id,u.personal_account_id,NOT (
 EXISTS(SELECT 1 FROM core_billing.opening_imports o WHERE o.account_id=u.personal_account_id)
 OR EXISTS(SELECT 1 FROM core_migration.import_runs i WHERE
 coalesce(i.manifest->>'scope','identity-only')='identity-only'
 OR (i.manifest->>'scope'='legacy-cutover-identity.v1'
 AND i.manifest->'registration_times' ? u.personal_account_id::text))
 ) AS native_complete FROM core_identity.users u WHERE u.id=ANY($1)
), snapshot AS MATERIALIZED (
 SELECT s.document,s.mapping,s.native_credential_floor,s.snapshot_observed_at
 FROM core_history.connection_snapshot s
 WHERE s.target_database=current_database()
 AND s.target_database_oid=(SELECT oid FROM pg_database WHERE datname=current_database())
 AND EXISTS(SELECT FROM pg_db_role_setting p CROSS JOIN LATERAL unnest(p.setconfig) c
   WHERE p.setdatabase=s.target_database_oid AND p.setrole=0 AND c='lmm.migration_instance_nonce='||s.target_nonce)
), mapped AS MATERIALIZED (
 SELECT u.id,m.source_user_id,s.native_credential_floor,s.snapshot_observed_at,
 su.created_at AS registered_at,s.document
 FROM users u CROSS JOIN snapshot s
 CROSS JOIN LATERAL jsonb_to_recordset(s.mapping->'users') m(source_user_id bigint,user_id bigint,account_id bigint)
 CROSS JOIN LATERAL jsonb_to_recordset(s.document->'users') su(id bigint,created_at bigint)
 WHERE m.user_id=u.id AND m.account_id=u.personal_account_id AND su.id=m.source_user_id
), tokens AS MATERIALIZED (
 SELECT m.id,t.oauth_managed,t.created_time,m.registered_at,m.snapshot_observed_at
 FROM mapped m CROSS JOIN LATERAL jsonb_to_recordset(m.document->'tokens') t(user_id bigint,oauth_managed boolean,created_time bigint)
 WHERE t.user_id=m.source_user_id
), historical AS MATERIALIZED (
 SELECT m.id,
 (SELECT min(t.created_time) FROM tokens t WHERE t.id=m.id AND t.oauth_managed=false AND t.created_time>0 AND t.created_time BETWEEN m.registered_at AND m.snapshot_observed_at) AS key_at,
 (SELECT min(at) FROM (
 SELECT t.created_time AS at FROM tokens t WHERE t.id=m.id AND t.oauth_managed=true AND t.created_time>0 AND t.created_time BETWEEN m.registered_at AND m.snapshot_observed_at
 UNION ALL SELECT g.created_at_ms/1000 FROM jsonb_to_recordset(m.document->'oauth_server_grants') g(user_id bigint,created_at_ms bigint)
 WHERE g.user_id=m.source_user_id AND g.created_at_ms/1000>0 AND g.created_at_ms/1000 BETWEEN m.registered_at AND m.snapshot_observed_at) events) AS oauth_at,
 m.registered_at>0 AND NOT EXISTS(SELECT FROM tokens t WHERE t.id=m.id AND
 (t.oauth_managed IS NULL OR (t.oauth_managed=false AND NOT coalesce(t.created_time BETWEEN m.registered_at AND m.snapshot_observed_at,false)))) AS key_complete,
 m.registered_at>0 AND NOT EXISTS(SELECT FROM tokens t WHERE t.id=m.id AND
 (t.oauth_managed IS NULL OR (t.oauth_managed=true AND NOT coalesce(t.created_time BETWEEN m.registered_at AND m.snapshot_observed_at,false))))
 AND NOT EXISTS(SELECT FROM jsonb_to_recordset(m.document->'oauth_server_grants') g(user_id bigint,created_at_ms bigint)
 WHERE g.user_id=m.source_user_id AND NOT coalesce(g.created_at_ms/1000 BETWEEN m.registered_at AND m.snapshot_observed_at,false)) AS oauth_complete,
 (SELECT min(a.created_at) FROM jsonb_to_recordset(m.document->'developer_access_requests') a(user_id bigint,created_at bigint)
 WHERE a.user_id=m.source_user_id AND a.created_at>0 AND a.created_at BETWEEN m.registered_at AND m.snapshot_observed_at) AS application_at,
 (SELECT min(a.reviewed_at) FROM jsonb_to_recordset(m.document->'developer_access_requests') a(user_id bigint,status text,reviewed_at bigint)
 WHERE a.user_id=m.source_user_id AND a.status='approved' AND a.reviewed_at>0 AND a.reviewed_at BETWEEN m.registered_at AND m.snapshot_observed_at) AS approval_at,
 m.registered_at>0 AND NOT EXISTS(SELECT FROM jsonb_to_recordset(m.document->'developer_access_requests') a(user_id bigint,created_at bigint)
 WHERE a.user_id=m.source_user_id AND NOT coalesce(a.created_at BETWEEN m.registered_at AND m.snapshot_observed_at,false)) AS application_complete,
 m.registered_at>0 AND NOT EXISTS(SELECT FROM jsonb_to_recordset(m.document->'developer_access_requests') a(user_id bigint,status text,reviewed_at bigint)
 WHERE a.user_id=m.source_user_id AND (a.status IS NULL OR (a.status='approved' AND NOT coalesce(a.reviewed_at BETWEEN m.registered_at AND m.snapshot_observed_at,false)))) AS approval_complete
 FROM mapped m
), native AS (
 SELECT u.id,
 (SELECT floor(extract(epoch FROM min(c.created_at)))::bigint FROM core_identity.credentials c
 WHERE c.user_id=u.id AND c.kind='api_key' AND c.created_at>to_timestamp(0)
 AND ((m.id IS NOT NULL AND c.id>m.native_credential_floor) OR (m.id IS NULL AND u.native_complete))) AS key_at,
 (SELECT floor(extract(epoch FROM min(f.created_at)))::bigint FROM (
 SELECT f.created_at FROM core_oauth.families f WHERE f.user_id=u.id AND f.issuance_kind='browser' AND f.created_at>to_timestamp(0)
 UNION ALL SELECT f.created_at FROM core_market_oauth.families f WHERE f.user_id=u.id AND f.created_at>to_timestamp(0)) f) AS oauth_at
 FROM users u LEFT JOIN mapped m ON m.id=u.id
)
SELECT u.id AS user_id,
 least(n.key_at,CASE WHEN h.key_at>=m.registered_at THEN h.key_at END) AS key_at,
 least(n.oauth_at,CASE WHEN h.oauth_at>=m.registered_at THEN h.oauth_at END) AS oauth_at,
 (u.native_complete OR coalesce(h.key_complete AND h.oauth_complete,false)) AS complete,
 CASE WHEN h.application_at>=m.registered_at THEN h.application_at END AS application_at,
 CASE WHEN h.approval_at>=m.registered_at THEN h.approval_at END AS approval_at,
 coalesce(h.application_complete,false) AS application_complete,
 coalesce(h.approval_complete,false) AS approval_complete,
 CASE WHEN m.id IS NOT NULL THEN 'retained_snapshot_and_native' WHEN u.native_complete THEN 'native' ELSE 'incomplete_legacy' END AS history_source
 FROM users u JOIN native n ON n.id=u.id LEFT JOIN mapped m ON m.id=u.id LEFT JOIN historical h ON h.id=u.id ORDER BY u.id"#;
