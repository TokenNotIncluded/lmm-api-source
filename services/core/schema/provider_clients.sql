-- Component78: explicit fresh installation or pinned owner77->78 only.
-- Historical component SQL is immutable; these exact catalog changes are the delta.

CREATE OR REPLACE FUNCTION core_oauth.client_scope_set(client_ TEXT,values_ TEXT[],initial_ BOOLEAN) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE filtered_ TEXT[];
BEGIN
 IF client_ IN ('lmm-dsh','lmm-opencode','lmm-codewhale','lmm-vscode','lmm-zed') THEN
  IF cardinality(values_)<>(SELECT count(DISTINCT v) FROM unnest(values_) v) THEN RETURN FALSE; END IF;
  IF initial_ THEN
   RETURN cardinality(values_) BETWEEN 4 AND 6
    AND values_ @> ARRAY['catalog:read','balance:read','usage:read','models:invoke']::text[]
    AND values_ <@ ARRAY['catalog:read','balance:read','usage:read','models:invoke','mcp:bounties','mcp:drawing']::text[];
  END IF;
  IF cardinality(values_) NOT BETWEEN 2 AND 68
     OR NOT (values_ && ARRAY['catalog:read','balance:read','usage:read','models:invoke']::text[]) THEN RETURN FALSE; END IF;
  filtered_:=array_remove(array_remove(values_,'usage:read'),'models:invoke');
  IF NOT (filtered_ && ARRAY['catalog:read','balance:read']::text[]) THEN filtered_:=filtered_||'catalog:read'::text; END IF;
  RETURN core_oauth.readonly_scope_set(filtered_);
 END IF;
 IF client_ IN ('lmm-cc-switch','lmm-magpie') THEN
  IF cardinality(values_)<>(SELECT count(DISTINCT v) FROM unnest(values_) v) THEN RETURN FALSE; END IF;
  IF initial_ THEN RETURN cardinality(values_)=4 AND values_ @> ARRAY['balance:read','catalog:read','models:invoke','usage:read']::text[]; END IF;
  RETURN values_ @> ARRAY['catalog:read','models:invoke']::text[]
    AND core_oauth.readonly_scope_set(array_remove(array_remove(values_,'models:invoke'),'usage:read'));
 END IF;
 IF cardinality(values_)<>(SELECT count(DISTINCT v) FROM unnest(values_) v) THEN RETURN FALSE; END IF;
 IF initial_ THEN
  IF client_='lmm' THEN RETURN cardinality(values_)=2 AND values_ @> ARRAY['catalog:read','balance:read']::text[]; END IF;
  RETURN client_='lmm-pi' AND cardinality(values_) BETWEEN 4 AND 7
   AND values_ @> ARRAY['catalog:read','balance:read','usage:read','models:invoke']::text[]
   AND values_ <@ ARRAY['catalog:read','balance:read','usage:read','models:invoke','mcp:bounties','mcp:drawing','remote:control']::text[];
 END IF;
 IF client_='lmm' THEN RETURN core_oauth.readonly_scope_set(values_); END IF;
 IF client_<>'lmm-pi' OR cardinality(values_) NOT BETWEEN 2 AND 68
    OR NOT (values_ && ARRAY['catalog:read','balance:read','usage:read','models:invoke']::text[]) THEN RETURN FALSE; END IF;
 filtered_:=array_remove(array_remove(values_,'usage:read'),'models:invoke');
 IF NOT (filtered_ && ARRAY['catalog:read','balance:read']::text[]) THEN filtered_:=filtered_||'catalog:read'::text; END IF;
 RETURN core_oauth.readonly_scope_set(filtered_);
END $$;

ALTER TABLE core_oauth.families DROP CONSTRAINT families_client_id_check;
ALTER TABLE core_oauth.families ADD CONSTRAINT families_client_id_check CHECK(client_id IN ('lmm','lmm-pi','lmm-cc-switch','lmm-magpie','lmm-dsh','lmm-opencode','lmm-codewhale','lmm-vscode','lmm-zed'));

ALTER TABLE core_oauth.authorization_requests DROP CONSTRAINT authorization_requests_client_id_check;
ALTER TABLE core_oauth.authorization_requests ADD CONSTRAINT authorization_requests_client_id_check CHECK(client_id IN ('lmm','lmm-pi','lmm-cc-switch','lmm-magpie','lmm-dsh','lmm-opencode','lmm-codewhale','lmm-vscode','lmm-zed'));

ALTER TABLE core_oauth.authorization_codes DROP CONSTRAINT authorization_codes_client_id_check;
ALTER TABLE core_oauth.authorization_codes ADD CONSTRAINT authorization_codes_client_id_check CHECK(client_id IN ('lmm','lmm-pi','lmm-cc-switch','lmm-magpie','lmm-dsh','lmm-opencode','lmm-codewhale','lmm-vscode','lmm-zed'));

ALTER TABLE core_oauth.families DROP CONSTRAINT families_check1;
ALTER TABLE core_oauth.families ADD CONSTRAINT families_check1 CHECK ((issuance_kind = 'foundation'::text AND purpose = 'foundation'::text AND budget_credits IS NULL AND client_id = 'lmm-pi'::text AND cardinality(scopes) >= 1 AND cardinality(scopes) <= 7 AND scopes <@ ARRAY['catalog:read'::text, 'balance:read'::text, 'usage:read'::text, 'models:invoke'::text, 'mcp:bounties'::text, 'mcp:drawing'::text, 'remote:control'::text] OR issuance_kind = 'browser'::text AND client_id = 'lmm'::text AND purpose = 'read_only'::text AND budget_credits IS NULL AND core_oauth.readonly_scope_set(scopes) OR issuance_kind = 'browser'::text AND client_id = 'lmm-pi'::text AND core_oauth.client_scope_set(client_id, scopes, false) AND (purpose = 'normal_model'::text AND budget_credits IS NOT NULL OR purpose = 'honeypot_only'::text AND budget_credits IS NULL)) OR (client_id IN ('lmm-cc-switch','lmm-magpie','lmm-dsh','lmm-opencode','lmm-codewhale','lmm-vscode','lmm-zed') AND issuance_kind='browser' AND purpose='normal_model' AND budget_credits IS NOT NULL AND core_oauth.client_scope_set(client_id,scopes,false)));

ALTER TABLE core_charging.requests DROP CONSTRAINT requests_oauth_source_check;
ALTER TABLE core_charging.requests ADD CONSTRAINT requests_oauth_source_check CHECK (oauth_source IS NULL OR (jsonb_typeof(oauth_source) = 'object'::text AND oauth_source ?& ARRAY['grant_id'::text, 'access_id'::text, 'client_id'::text, 'issuer'::text, 'resource'::text, 'user_id'::text, 'personal_account_id'::text, 'user_version'::text, 'consent_version'::text, 'scopes'::text, 'expires_at'::text, 'budget_credits'::text] AND (oauth_source - ARRAY['grant_id'::text, 'access_id'::text, 'client_id'::text, 'issuer'::text, 'resource'::text, 'user_id'::text, 'personal_account_id'::text, 'user_version'::text, 'consent_version'::text, 'scopes'::text, 'expires_at'::text, 'budget_credits'::text]) = '{}'::jsonb AND (oauth_source ->> 'client_id'::text) = ANY (ARRAY['lmm-pi'::text,'lmm-cc-switch'::text,'lmm-magpie'::text,'lmm-dsh'::text,'lmm-opencode'::text,'lmm-codewhale'::text,'lmm-vscode'::text,'lmm-zed'::text]) AND (oauth_source ->> 'grant_id'::text) ~ '^lmm_gr_[0-9a-f]{64}$'::text AND (oauth_source ->> 'access_id'::text) ~ '^lmm_ac_[0-9a-f]{64}$'::text AND jsonb_typeof(oauth_source -> 'scopes'::text) = 'array'::text AND jsonb_typeof(oauth_source -> 'grant_id'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'access_id'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'client_id'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'issuer'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'resource'::text) = 'string'::text AND (oauth_source -> 'scopes'::text) ? 'models:invoke'::text AND jsonb_array_length(oauth_source -> 'scopes'::text) >= 2 AND jsonb_array_length(oauth_source -> 'scopes'::text) <= 68 AND (oauth_source ->> 'expires_at'::text) ~ '^[1-9][0-9]{0,11}$'::text AND ((oauth_source ->> 'expires_at'::text)::numeric) <= '253402300799'::bigint::numeric AND jsonb_typeof(oauth_source -> 'budget_credits'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'user_id'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'personal_account_id'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'user_version'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'consent_version'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'expires_at'::text) = 'number'::text AND ((oauth_source ->> 'budget_credits'::text)::numeric) >= 1::numeric AND ((oauth_source ->> 'budget_credits'::text)::numeric) <= '9007199254740991'::bigint::numeric) IS TRUE);

CREATE TABLE core_meta.provider_clients_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER provider_clients_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.provider_clients_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER provider_clients_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.provider_clients_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.provider_clients_upgrades FROM PUBLIC;

-- Preserve the source-bound importer on the current assembly.
CREATE OR REPLACE FUNCTION core_history.guard_connection_snapshot() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE d JSONB:=NEW.document_utf8::jsonb; m JSONB:=NEW.mapping_utf8::jsonb;
 row_ JSONB; user_ JSONB; source_id_ BIGINT; nonce_ TEXT; expected_ BIGINT;
BEGIN
 IF current_user<>pg_get_userbyid((SELECT relowner FROM pg_class WHERE oid='core_history.connection_snapshot'::regclass))
 OR current_setting('transaction_isolation')<>'serializable' THEN
  RAISE EXCEPTION 'connection import requires serializable offline owner' USING ERRCODE='42501';
 END IF;
 IF NEW.target_database<>current_database()
 OR NEW.target_database_oid<>(SELECT oid FROM pg_database WHERE datname=current_database())
 OR NEW.catalog_sha256<>(SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton)
 OR (SELECT count(*) FROM core_meta.component_contract)<>78 THEN
  RAISE EXCEPTION 'connection target contract mismatch';
 END IF;
 SELECT substring(c FROM length('lmm.migration_instance_nonce=')+1) INTO nonce_
 FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) c
 WHERE s.setdatabase=NEW.target_database_oid AND s.setrole=0 AND c LIKE 'lmm.migration_instance_nonce=%';
 IF nonce_ IS DISTINCT FROM NEW.target_nonce THEN RAISE EXCEPTION 'connection target nonce mismatch'; END IF;
 IF NOT EXISTS(SELECT FROM core_billing.opening_import_runs r WHERE r.migration_id=NEW.opening_migration_id
 AND r.source_archive_sha256=NEW.source_archive_sha256 AND r.mapping_sha256=NEW.opening_mapping_sha256)
 OR NOT EXISTS(SELECT FROM core_migration.import_runs i WHERE i.migration_id=NEW.identity_migration_id
 AND i.source_archive_sha256=NEW.source_archive_sha256 AND i.manifest->>'scope'='legacy-cutover-identity.v1'
 AND i.manifest->>'source_archive_sha256'=NEW.source_archive_sha256) THEN
  RAISE EXCEPTION 'connection original migration mismatch';
 END IF;
 IF NOT coalesce(d->>'contract'='legacy.connection-source.v1'
 AND NEW.metadata->>'contract'='legacy.connection-export-binding.v1'
 AND NEW.metadata->>'source_archive_sha256'=NEW.source_archive_sha256
 AND NEW.metadata->>'export_file_sha256'=NEW.export_file_sha256
 AND NEW.metadata->>'source_schema'=d->>'source_schema'
 AND (NEW.metadata->>'snapshot_observed_at')::bigint=NEW.snapshot_observed_at
 AND NEW.snapshot_observed_at<=(d->>'observed_at')::bigint
 AND length(NEW.metadata->>'source_snapshot_id') BETWEEN 1 AND 200
 AND d->>'coverage'='all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked'
 AND NEW.metadata->>'coverage'=d->>'coverage'
 AND m->>'contract'='legacy.connection-mapping.v1',false) THEN
  RAISE EXCEPTION 'connection source coverage/binding mismatch';
 END IF;
 FOREACH row_ IN ARRAY ARRAY[d->'users',d->'tokens',d->'oauth_server_grants',d->'developer_access_requests',m->'users',m->'omitted_users'] LOOP
  IF jsonb_typeof(row_) IS DISTINCT FROM 'array' THEN RAISE EXCEPTION 'connection source arrays required'; END IF;
 END LOOP;
 IF (d->'counts'->>'users')::bigint IS DISTINCT FROM jsonb_array_length(d->'users')::bigint
 OR (d->'counts'->>'tokens')::bigint IS DISTINCT FROM jsonb_array_length(d->'tokens')::bigint
 OR (d->'counts'->>'oauth_server_grants')::bigint IS DISTINCT FROM jsonb_array_length(d->'oauth_server_grants')::bigint
 OR (d->'counts'->>'developer_access_requests')::bigint IS DISTINCT FROM jsonb_array_length(d->'developer_access_requests')::bigint
 OR jsonb_array_length(d->'users')<>jsonb_array_length(m->'users')+jsonb_array_length(m->'omitted_users') THEN
  RAISE EXCEPTION 'connection retained cohort/count mismatch';
 END IF;
 IF EXISTS(SELECT FROM jsonb_array_elements(d->'users') u GROUP BY u->>'id' HAVING count(*)<>1)
 OR EXISTS(SELECT FROM (SELECT v->>'source_user_id' id FROM jsonb_array_elements(m->'users') v
 UNION ALL SELECT v->>'source_user_id' FROM jsonb_array_elements(m->'omitted_users') v) ids GROUP BY id HAVING count(*)<>1)
 OR EXISTS(SELECT FROM jsonb_array_elements(m->'users') v GROUP BY v->>'user_id' HAVING count(*)<>1) THEN
  RAISE EXCEPTION 'connection duplicate mapping';
 END IF;
 FOR user_ IN SELECT value FROM jsonb_array_elements(d->'users') LOOP
  source_id_:=(user_->>'id')::bigint;
  SELECT v INTO row_ FROM jsonb_array_elements(m->'users') v WHERE (v->>'source_user_id')::bigint=source_id_;
  IF row_ IS NULL THEN
   IF NOT EXISTS(SELECT FROM jsonb_array_elements(m->'omitted_users') v
     WHERE (v->>'source_user_id')::bigint=source_id_ AND length(btrim(v->>'reason')) BETWEEN 1 AND 500) THEN
    RAISE EXCEPTION 'connection source user missing from mapping';
   END IF;
  ELSE
   IF row_->>'opening_entity_key' NOT IN ('user:'||source_id_::text,'legacy.users/'||source_id_::text)
   OR NOT EXISTS(SELECT FROM core_identity.users u
    JOIN core_identity.accounts a ON a.id=u.personal_account_id
    JOIN core_billing.opening_imports o ON o.account_id=a.id
    JOIN core_migration.import_runs i ON i.migration_id=NEW.identity_migration_id
    WHERE u.id=(row_->>'user_id')::bigint AND a.id=(row_->>'account_id')::bigint
    AND o.migration_id=NEW.opening_migration_id AND o.source_entity_key=row_->>'opening_entity_key'
    AND i.manifest->'registration_times'->(a.id::text)=to_jsonb(coalesce((user_->>'created_at')::bigint,0))
    AND floor(extract(epoch FROM a.created_at))::bigint=coalesce((user_->>'created_at')::bigint,0)) THEN
    RAISE EXCEPTION 'connection personal opening/identity mapping mismatch';
   END IF;
  END IF;
 END LOOP;
 SELECT coalesce(max(id),0) INTO expected_ FROM core_identity.credentials;
 IF NEW.native_credential_floor<>expected_ THEN RAISE EXCEPTION 'connection credential boundary changed'; END IF;
 -- The supported cutover importer preserves token IDs. Check every existing
 -- api_key against its original row and mapped owner, including managed/revoked.
 -- Native keys must be created AFTER this import, never guessed from row status.
 IF EXISTS(SELECT FROM core_identity.credentials c WHERE c.kind='api_key' AND NOT EXISTS(
  SELECT FROM jsonb_array_elements(d->'tokens') t JOIN jsonb_array_elements(m->'users') v
  ON (v->>'source_user_id')::bigint=(t->>'user_id')::bigint
  WHERE (t->>'id')::bigint=c.id AND (v->>'user_id')::bigint=c.user_id
  AND (v->>'account_id')::bigint=c.owner_account_id
  AND coalesce((t->>'created_time')::bigint,0)=floor(extract(epoch FROM c.created_at))::bigint)) THEN
  RAISE EXCEPTION 'connection unsupported or native credential before source boundary';
 END IF;
 RETURN NEW;
END $$;

