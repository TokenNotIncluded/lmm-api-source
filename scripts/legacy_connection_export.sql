-- Local frozen legacy source only. psql -X -qAt -v ON_ERROR_STOP=1 -f this.sql
-- Set search_path to the legacy schema with PGOPTIONS (one schema, not public fallback).
-- Redirect stdout to a private JSON file; no live keys or token digests are read.
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
WITH users_ AS MATERIALIZED (
 SELECT id,created_at FROM users ORDER BY id
), tokens_ AS MATERIALIZED (
 SELECT id,user_id,oauth_managed,created_time,deleted_at FROM tokens ORDER BY id
), grants_ AS MATERIALIZED (
 SELECT id,user_id,created_at_ms,revoked_at_ms FROM oauth_server_grants ORDER BY id
), applications_ AS MATERIALIZED (
 SELECT id,user_id,created_at,reviewed_at,status FROM developer_access_requests ORDER BY id
)
SELECT jsonb_build_object(
 'contract','legacy.connection-source.v1',
 'source_schema',current_schema(),
 'observed_at',floor(extract(epoch FROM transaction_timestamp()))::bigint,
 'coverage','all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked',
 'counts',jsonb_build_object('users',(SELECT count(*) FROM users_),
                           'tokens',(SELECT count(*) FROM tokens_),
                           'oauth_server_grants',(SELECT count(*) FROM grants_),
                           'developer_access_requests',(SELECT count(*) FROM applications_)),
 'users',coalesce((SELECT jsonb_agg(to_jsonb(u) ORDER BY id) FROM users_ u),'[]'::jsonb),
 'tokens',coalesce((SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM tokens_ t),'[]'::jsonb),
 'oauth_server_grants',coalesce((SELECT jsonb_agg(to_jsonb(g) ORDER BY id) FROM grants_ g),'[]'::jsonb),
 'developer_access_requests',coalesce((SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM applications_ a),'[]'::jsonb)
);
COMMIT;
