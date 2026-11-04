-- Component56: dedicated platform advertisements. Prior components remain fixed.
CREATE SCHEMA core_directory_ads;
REVOKE ALL ON SCHEMA core_directory_ads FROM PUBLIC;
ALTER TABLE core_charging.external_consumption DROP CONSTRAINT external_consumption_domain_check;
ALTER TABLE core_charging.external_consumption ADD CONSTRAINT external_consumption_domain_check CHECK(domain IN ('commerce','toolmarket','bounty','directory_ad'));
CREATE TABLE core_directory_ads.quotes (
 id text PRIMARY KEY,
 principal text NOT NULL,
 request_id text NOT NULL,
 user_id bigint NOT NULL REFERENCES core_identity.users(id),
 account_id bigint NOT NULL REFERENCES core_identity.accounts(id),
 command jsonb NOT NULL,
 content_sha256 text NOT NULL,
 quota bigint NOT NULL CHECK(quota>0),
 created_at bigint NOT NULL,
 expires_at bigint NOT NULL,
 confirmation_digest bytea,
 confirmation_sid text,
 confirmed_sid text,
 confirmed_at bigint,
 UNIQUE(principal,request_id)
);
CREATE TABLE core_directory_ads.payments (
 id text PRIMARY KEY REFERENCES core_directory_ads.quotes(id),
 principal text NOT NULL,
 request_id text NOT NULL,
 user_id bigint NOT NULL REFERENCES core_identity.users(id),
 command jsonb NOT NULL,
 receipt jsonb NOT NULL,
 journal_id bigint REFERENCES core_billing.ledger_journals(id),
 paid_at bigint,
 expires_at bigint,
 UNIQUE(principal,request_id)
);
CREATE TABLE core_directory_ads.refunds (
 principal text NOT NULL,
 request_id text NOT NULL,
 payment_id text NOT NULL UNIQUE REFERENCES core_directory_ads.payments(id),
 command jsonb NOT NULL,
 accepted_at bigint NOT NULL,
 accepted_by bigint NOT NULL REFERENCES core_identity.users(id),
 receipt jsonb NOT NULL,
 journal_id bigint REFERENCES core_billing.ledger_journals(id),
 PRIMARY KEY(principal,request_id)
);
CREATE FUNCTION core_directory_ads.protect_fact() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'advertising facts cannot be deleted'; END IF;
 IF TG_TABLE_NAME='payments' THEN RAISE EXCEPTION 'advertising payments are immutable'; END IF;
 IF TG_TABLE_NAME='quotes' AND (to_jsonb(NEW)-ARRAY['confirmation_digest','confirmation_sid','confirmed_sid','confirmed_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['confirmation_digest','confirmation_sid','confirmed_sid','confirmed_at']) THEN
  RAISE EXCEPTION 'advertising quote binding is immutable';
 END IF;
 IF TG_TABLE_NAME='refunds' AND ((to_jsonb(NEW)-ARRAY['receipt','journal_id']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['receipt','journal_id']) OR to_jsonb(OLD)->>'journal_id' IS NOT NULL) THEN
  RAISE EXCEPTION 'advertising refund binding is immutable';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER quotes_immutable BEFORE UPDATE OR DELETE ON core_directory_ads.quotes FOR EACH ROW EXECUTE FUNCTION core_directory_ads.protect_fact();
CREATE TRIGGER payments_immutable BEFORE UPDATE OR DELETE ON core_directory_ads.payments FOR EACH ROW EXECUTE FUNCTION core_directory_ads.protect_fact();
CREATE TRIGGER refunds_immutable BEFORE UPDATE OR DELETE ON core_directory_ads.refunds FOR EACH ROW EXECUTE FUNCTION core_directory_ads.protect_fact();
REVOKE ALL ON ALL TABLES IN SCHEMA core_directory_ads FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_directory_ads FROM PUBLIC;

-- Preserve the reviewed history guard with the exact new assembly count.
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
 OR (SELECT count(*) FROM core_meta.component_contract)<>56 THEN
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
