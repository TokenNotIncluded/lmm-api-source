-- Validate each owner receipt once, independent of the number of users on a page.
WITH history_identity_receipts AS MATERIALIZED (
 SELECT r.migration_id, r.manifest->'mapping' AS mapping,
   r.manifest->'source_user_facts' AS source_user_facts,
   r.manifest#>>'{source_snapshot,sha256}' AS source_sha256,
   r.manifest#>>'{source_snapshot,metadata,schema}' AS source_schema,
   r.manifest#>'{source_snapshot,metadata,observed_at}' AS observed_at,
   r.manifest#>'{target,before_identity_state,users,max_id}' AS previous_user_id,
   r.manifest#>'{target,before_identity_state,accounts,max_id}' AS previous_account_id
 FROM core_migration.import_runs r
 CROSS JOIN LATERAL (
   SELECT count(*) AS size,
     count(DISTINCT m.value->'source_user_id') AS source_users,
     count(DISTINCT m.value->'target_user_id') AS target_users,
     count(DISTINCT m.value->'target_personal_account_id') AS accounts,
     bool_and(CASE WHEN jsonb_typeof(m.value)='object' THEN
       m.value - ARRAY['source_user_id','target_user_id','target_personal_account_id']='{}'::jsonb
       AND jsonb_typeof(m.value->'source_user_id')='number'
       AND jsonb_typeof(m.value->'target_user_id')='number'
       AND jsonb_typeof(m.value->'target_personal_account_id')='number'
       AND m.value->>'source_user_id' ~ '^[1-9][0-9]*$'
       AND m.value->>'target_user_id' ~ '^[1-9][0-9]*$'
       AND m.value->>'target_personal_account_id' ~ '^[1-9][0-9]*$'
       ELSE false END) AS valid,
     encode(sha256(convert_to('[' || string_agg(
       '{"source_user_id":' || (m.value->'source_user_id')::text
       || ',"target_personal_account_id":' || (m.value->'target_personal_account_id')::text
       || ',"target_user_id":' || (m.value->'target_user_id')::text || '}',
       ',' ORDER BY m.position) || ']', 'UTF8')), 'hex') AS mapping_sha256
   FROM jsonb_array_elements(CASE WHEN jsonb_typeof(r.manifest->'mapping')='array'
     THEN r.manifest->'mapping' ELSE '[]'::jsonb END) WITH ORDINALITY m(value,position)
 ) checked
 WHERE r.manifest->>'scope'='history-only-identity-supplement.v1'
   AND r.manifest->'format_version'='1'::jsonb
   AND checked.valid AND checked.size>0 AND checked.size=checked.source_users
   AND checked.size=checked.target_users AND checked.size=checked.accounts
   AND checked.mapping_sha256=r.manifest->>'mapping_sha256'
   AND r.manifest#>>'{target,instance,database}'=current_database()
   AND r.manifest#>'{target,instance,database_oid}'=
     to_jsonb((SELECT oid::bigint FROM pg_database WHERE datname=current_database()))
   AND r.manifest->'target_policy' @> '{"accounts_active":false,"users_active":false,
     "platform_role":"user","auth_version":1,"credentials_imported":false,
     "wallet_opening_imported":false}'::jsonb
), history_identity_mappings AS MATERIALIZED (
 SELECT r.migration_id, m->'source_user_id' AS source_user_id,
   m->'target_user_id' AS target_user_id,
   m->'target_personal_account_id' AS target_account_id
 FROM history_identity_receipts r
 CROSS JOIN LATERAL jsonb_array_elements(r.mapping) m
), history_identity_facts AS MATERIALIZED (
 SELECT DISTINCT r.migration_id, f#>'{facts,id}' AS source_user_id
 FROM history_identity_receipts r
 CROSS JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(r.source_user_facts)='array'
   THEN r.source_user_facts ELSE '[]'::jsonb END) f
 WHERE f->>'source_record_sha256' ~ '^[a-f0-9]{64}$'
), history_only AS (
 SELECT u.id
 FROM history_identity_mappings m
 JOIN history_identity_receipts r ON r.migration_id=m.migration_id
 JOIN history_identity_facts f ON f.migration_id=m.migration_id AND f.source_user_id=m.source_user_id
 JOIN core_identity.users u ON to_jsonb(u.id)=m.target_user_id
 JOIN core_identity.accounts a ON a.id=u.personal_account_id AND to_jsonb(a.id)=m.target_account_id
 JOIN core_identity.user_profiles p ON p.user_id=u.id
 JOIN core_history.sources s ON s.source_archive_sha256=r.source_sha256
   AND s.source_namespace='legacy.logs:' || r.source_schema
   AND to_jsonb(s.observed_at)=r.observed_at
   AND s.complete AND s.contract_version='core.history-source.v1'
 JOIN core_history.scope_receipts h ON h.source_id=s.source_id AND h.user_id=u.id
   AND h.owner_account_id=a.id AND h.token_id IS NULL AND h.complete
   AND h.source_archive_sha256=s.source_archive_sha256 AND h.mapping_sha256=s.mapping_sha256
 WHERE NOT u.active AND NOT a.active AND a.kind='personal' AND a.service_level=0
   AND u.platform_role='user' AND u.auth_version=1
   AND NOT p.password_login_allowed AND NOT p.email_login_allowed AND NOT p.legacy_bcrypt_allowed
   AND NOT p.second_factor_required AND p.email=''
   AND jsonb_typeof(r.previous_user_id)='number'
   AND to_jsonb(u.id)>r.previous_user_id
   AND jsonb_typeof(r.previous_account_id)='number'
   AND to_jsonb(a.id)>r.previous_account_id
   AND NOT EXISTS(SELECT 1 FROM core_identity.credentials c
     WHERE c.user_id=u.id OR c.owner_account_id=a.id)
   AND NOT EXISTS(SELECT 1 FROM core_identity.password_logins l WHERE l.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_identity.oauth_identities o WHERE o.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_identity.web_sessions w WHERE w.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_identity.mfa_factors f WHERE f.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_identity.email_addresses e WHERE e.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_passkeys.credentials k WHERE k.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_abuse.registration_observations o WHERE o.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_identity.audit e WHERE e.actor_user_id=u.id
     AND e.action IN ('user.register','user.bootstrap','installation.first-owner')
     AND e.target->>'user_id'=u.id::text)
   AND NOT EXISTS(SELECT 1 FROM core_billing.opening_imports o WHERE o.account_id=a.id)
   AND NOT EXISTS(SELECT 1 FROM core_billing.account_opening_counters c
     WHERE c.account_id=a.id OR c.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_history.finance_users f
     WHERE f.account_id=a.id OR f.user_id=u.id)
   AND NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts l WHERE l.owner_account_id=a.id)
   AND NOT EXISTS(SELECT 1 FROM core_charging.requests q
     WHERE q.actor_user_id=u.id OR q.owner_account_id=a.id OR q.payer_account_id=a.id)
   AND NOT EXISTS(SELECT 1 FROM core_payments.intents i
     WHERE i.created_by_user_id=u.id OR i.account_id=a.id)
)
