-- Run read-only against the new Core55 target after identity/opening/key imports.
-- No source PG or existing candidate is contacted by this script.
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT jsonb_build_object(
 'contract','legacy.connection-target.v1',
 'target_database',current_database(),
 'target_database_oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),
 'target_nonce',(SELECT substring(c FROM length('lmm.migration_instance_nonce=')+1)
   FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) c
   WHERE s.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database())
     AND s.setrole=0 AND c LIKE 'lmm.migration_instance_nonce=%'),
 'catalog_sha256',(SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton),
 'component_count',(SELECT count(*) FROM core_meta.component_contract),
 'native_credential_floor',(SELECT coalesce(max(id),0) FROM core_identity.credentials));
COMMIT;
