-- Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
-- Offline continuation of the fixed cc52 source. Source decisions and risk
-- evidence are not native model decisions, Google proofs or ledger journals.
CREATE TABLE core_gift.legacy_installations (
 namespace text PRIMARY KEY CHECK(length(namespace) BETWEEN 1 AND 200),
 singleton boolean NOT NULL UNIQUE DEFAULT true CHECK(singleton),
 source_revision text NOT NULL CHECK(source_revision='cc52aa195c6f2a01d703a1a407be73c31b89867d'),
 source_archive_sha256 text NOT NULL CHECK(source_archive_sha256 ~ '^[0-9a-f]{64}$'),
 source_export_sha256 text NOT NULL CHECK(source_export_sha256 ~ '^[0-9a-f]{64}$'),
 source_export_proof_sha256 text NOT NULL CHECK(source_export_proof_sha256 ~ '^[0-9a-f]{64}$'),
 identity_scope text NOT NULL CHECK(length(identity_scope) BETWEEN 1 AND 200),
 identity_validator_sha256 text NOT NULL CHECK(identity_validator_sha256 ~ '^[0-9a-f]{64}$'),
 identity_material_sha256 text NOT NULL CHECK(identity_material_sha256 ~ '^[0-9a-f]{64}$'),
 identity_receipt_sha256 text NOT NULL CHECK(identity_receipt_sha256 ~ '^[0-9a-f]{64}$'),
 identity_mapping_sha256 text NOT NULL CHECK(identity_mapping_sha256 ~ '^[0-9a-f]{64}$'),
 source_observed_at bigint NOT NULL CHECK(source_observed_at>0),
 source_enabled boolean NOT NULL,
 inclusive_cap bigint NOT NULL CHECK(inclusive_cap BETWEEN 0 AND 9007199254740990),
 risk_secret bytea NOT NULL CHECK(octet_length(risk_secret) BETWEEN 1 AND 4096),
 oidc_issuer text CHECK(length(oidc_issuer) BETWEEN 1 AND 2048),
 input_sha256 text NOT NULL UNIQUE CHECK(input_sha256 ~ '^[0-9a-f]{64}$'),
 installed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_gift.legacy_bindings (
 namespace text NOT NULL REFERENCES core_gift.legacy_installations(namespace),
 source_user_id bigint NOT NULL CHECK(source_user_id>0),
 user_id bigint UNIQUE REFERENCES core_identity.users(id),
 account_id bigint UNIQUE REFERENCES core_identity.accounts(id),
 source_present boolean NOT NULL,
 PRIMARY KEY(namespace,source_user_id),
 CHECK((user_id IS NULL)=(account_id IS NULL)),
 CHECK(source_present OR user_id IS NULL)
);
CREATE TABLE core_gift.legacy_decisions (
 namespace text NOT NULL,
 source_id bigint NOT NULL CHECK(source_id>0),
 source_user_id bigint NOT NULL,
 gift_id text NOT NULL UNIQUE CHECK(gift_id ~ '^[0-9a-f]{64}$'),
 source_status text NOT NULL CHECK(source_status IN ('offered','claimed','declined')),
 source_quota bigint NOT NULL CHECK(source_quota>=0),
 amount_credits bigint NOT NULL CHECK(amount_credits BETWEEN 0 AND 9007199254740991),
 created_at bigint NOT NULL CHECK(created_at>0),
 claimed_at bigint NOT NULL CHECK(claimed_at>=0),
 source_row_sha256 text NOT NULL CHECK(source_row_sha256 ~ '^[0-9a-f]{64}$'),
 amount_basis_sha256 text NOT NULL CHECK(amount_basis_sha256 ~ '^[0-9a-f]{64}$'),
 PRIMARY KEY(namespace,source_id),
 UNIQUE(namespace,source_user_id),
 FOREIGN KEY(namespace,source_user_id) REFERENCES core_gift.legacy_bindings(namespace,source_user_id),
 CHECK((source_status='claimed' AND claimed_at>=created_at AND amount_credits>0)
    OR (source_status='offered' AND claimed_at=0 AND amount_credits>0)
    OR (source_status='declined' AND claimed_at=0))
);
CREATE TABLE core_gift.legacy_risk_memories (
 namespace text NOT NULL REFERENCES core_gift.legacy_installations(namespace),
 key_hash bytea NOT NULL CHECK(octet_length(key_hash)=32),
 kind text NOT NULL CHECK(kind IN ('identity','network')),
 decision_count bigint NOT NULL CHECK(decision_count>=0),
 window_started_at bigint NOT NULL CHECK(window_started_at>=0),
 updated_at bigint NOT NULL CHECK(updated_at>=0),
 source_row_sha256 text NOT NULL CHECK(source_row_sha256 ~ '^[0-9a-f]{64}$'),
 PRIMARY KEY(namespace,key_hash),
 UNIQUE(namespace,key_hash,kind)
);
-- Original profiles and bounded fingerprints remain identifiable by source ID,
-- including orphan records. A new native user has a separate native:<id> key.
CREATE TABLE core_gift.legacy_profiles (
 namespace text NOT NULL REFERENCES core_gift.legacy_installations(namespace),
 subject text NOT NULL CHECK(subject ~ '^(source|native):[1-9][0-9]*$'),
 identity_hash bytea NOT NULL CHECK(octet_length(identity_hash)=32),
 network_hash bytea NOT NULL CHECK(octet_length(network_hash)=32),
 observed_at bigint NOT NULL CHECK(observed_at>0),
 PRIMARY KEY(namespace,subject)
);
CREATE TABLE core_gift.legacy_fingerprints (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 namespace text NOT NULL REFERENCES core_gift.legacy_installations(namespace),
 subject text NOT NULL CHECK(subject ~ '^(source|native):[1-9][0-9]*$'),
 message_digest bytea NOT NULL CHECK(octet_length(message_digest)=32),
 token bytea NOT NULL CHECK(octet_length(token)=32),
 created_at bigint NOT NULL CHECK(created_at>0),
 UNIQUE(namespace,subject,message_digest,token)
);
CREATE INDEX legacy_gift_fingerprint_token ON core_gift.legacy_fingerprints(namespace,token,created_at,subject);
CREATE TABLE core_gift.legacy_claims (
 namespace text NOT NULL,
 source_id bigint NOT NULL,
 user_id bigint NOT NULL REFERENCES core_identity.users(id),
 account_id bigint NOT NULL REFERENCES core_identity.accounts(id),
 amount_credits bigint NOT NULL CHECK(amount_credits>0),
 claimed_at bigint NOT NULL CHECK(claimed_at>0),
 ledger_journal_id bigint NOT NULL UNIQUE REFERENCES core_billing.ledger_journals(id),
 PRIMARY KEY(namespace,source_id),
 FOREIGN KEY(namespace,source_id) REFERENCES core_gift.legacy_decisions(namespace,source_id)
);
CREATE TABLE core_gift.legacy_imports (
 namespace text PRIMARY KEY REFERENCES core_gift.legacy_installations(namespace),
 scope text NOT NULL CHECK(scope='current-source-gift-continuation-v1'),
 operation_id text NOT NULL UNIQUE CHECK(length(operation_id) BETWEEN 16 AND 120),
 input_sha256 text NOT NULL UNIQUE CHECK(input_sha256 ~ '^[0-9a-f]{64}$'),
 material_sha256 text NOT NULL CHECK(material_sha256 ~ '^[0-9a-f]{64}$'),
 counts jsonb NOT NULL CHECK(jsonb_typeof(counts)='object'),
 installed_by name NOT NULL,
 database_oid oid NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
-- Only stable current subjects are exposed; gift runtime does not receive
-- SELECT on federated provider secrets or unrelated authentication tables.
CREATE FUNCTION core_gift.legacy_oauth_subject(p_user_id bigint,p_oidc_issuer text)
RETURNS TABLE(provider text,subject text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT x.provider,x.subject FROM (
  SELECT CASE i.issuer WHEN 'https://github.com' THEN 1 WHEN 'https://connect.linux.do' THEN 3 ELSE 4 END AS priority,
   CASE i.issuer WHEN 'https://github.com' THEN 'github' WHEN 'https://connect.linux.do' THEN 'linuxdo' ELSE 'discord' END AS provider,i.subject
  FROM core_identity.oauth_identities i LEFT JOIN core_account_links.oauth_states s USING(issuer,subject)
  WHERE i.user_id=p_user_id AND i.issuer IN ('https://github.com','https://connect.linux.do','https://discord.com') AND s.revoked_at IS NULL
  UNION ALL
  SELECT CASE p.kind WHEN 'oidc' THEN 2 ELSE 6 END,p.kind,i.subject
  FROM core_federated_auth.identities i JOIN core_federated_auth.providers p ON p.id=i.provider_id
  WHERE i.user_id=p_user_id AND i.revoked_at IS NULL AND p.deleted_at IS NULL
   AND (p.kind='wechat' OR (p.kind='oidc' AND p.configuration->>'issuer'=p_oidc_issuer))
  UNION ALL
  SELECT 5,'telegram',b.subject FROM core_telegram_auth.bindings b WHERE b.user_id=p_user_id AND b.revoked_at IS NULL
 ) x WHERE p_user_id>0 ORDER BY x.priority,x.subject LIMIT 1
$$;
CREATE FUNCTION core_gift.check_legacy_binding() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.user_id IS NOT NULL AND NOT EXISTS(
  SELECT 1 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
  WHERE u.id=NEW.user_id AND a.id=NEW.account_id AND a.kind='personal'
 ) THEN RAISE EXCEPTION 'legacy gift owner binding mismatch' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER legacy_gift_owner_bound AFTER INSERT ON core_gift.legacy_bindings
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_gift.check_legacy_binding();
CREATE FUNCTION core_gift.check_legacy_decision() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.gift_id<>encode(sha256(convert_to('legacy-assistant-gift-v1','UTF8')||decode('00','hex')
   ||convert_to(NEW.namespace,'UTF8')||decode('00','hex')||convert_to(NEW.source_id::text,'UTF8')),'hex')
 OR EXISTS(SELECT FROM core_gift.legacy_bindings b JOIN core_gift.decisions d ON d.user_id=b.user_id
   WHERE b.namespace=NEW.namespace AND b.source_user_id=NEW.source_user_id) THEN
  RAISE EXCEPTION 'legacy gift identity or native decision overlap' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER legacy_gift_decision_bound BEFORE INSERT ON core_gift.legacy_decisions
 FOR EACH ROW EXECUTE FUNCTION core_gift.check_legacy_decision();
CREATE FUNCTION core_gift.reject_legacy_native_overlap() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF EXISTS(SELECT FROM core_gift.legacy_decisions d JOIN core_gift.legacy_bindings b USING(namespace,source_user_id)
   WHERE b.user_id=NEW.user_id) THEN
  RAISE EXCEPTION 'native gift overlaps retained legacy decision' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER gift_no_legacy_overlap BEFORE INSERT ON core_gift.decisions
 FOR EACH ROW EXECUTE FUNCTION core_gift.reject_legacy_native_overlap();
CREATE FUNCTION core_gift.check_legacy_claim() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NOT EXISTS(
  SELECT 1 FROM core_gift.legacy_decisions d
  JOIN core_gift.legacy_bindings b USING(namespace,source_user_id)
  JOIN core_identity.users u ON u.id=b.user_id AND u.personal_account_id=b.account_id
  JOIN core_billing.ledger_journals j ON j.id=NEW.ledger_journal_id
  JOIN core_billing.ledger_accounts a ON a.id=j.payee_id
  WHERE d.namespace=NEW.namespace AND d.source_id=NEW.source_id AND d.source_status='offered'
   AND b.user_id=NEW.user_id AND b.account_id=NEW.account_id
   AND d.amount_credits=NEW.amount_credits AND NEW.claimed_at>=d.created_at
   AND j.kind='gift' AND j.scope='core-assistant-legacy-gift' AND j.operation_key=d.gift_id
   AND j.actor_user_id=NEW.user_id AND a.owner_account_id=NEW.account_id
   AND j.amount_units=NEW.amount_credits AND j.payment_reference IS NULL
 ) THEN RAISE EXCEPTION 'legacy gift claim journal mismatch' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER legacy_gift_claim_bound AFTER INSERT ON core_gift.legacy_claims
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_gift.check_legacy_claim();
CREATE TRIGGER legacy_gift_installation_immutable BEFORE UPDATE OR DELETE ON core_gift.legacy_installations
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_binding_immutable BEFORE UPDATE OR DELETE ON core_gift.legacy_bindings
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_decision_immutable BEFORE UPDATE OR DELETE ON core_gift.legacy_decisions
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_memory_immutable BEFORE UPDATE OR DELETE ON core_gift.legacy_risk_memories
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_claim_immutable BEFORE UPDATE OR DELETE ON core_gift.legacy_claims
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_import_immutable BEFORE UPDATE OR DELETE ON core_gift.legacy_imports
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_installation_no_truncate BEFORE TRUNCATE ON core_gift.legacy_installations
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_binding_no_truncate BEFORE TRUNCATE ON core_gift.legacy_bindings
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_decision_no_truncate BEFORE TRUNCATE ON core_gift.legacy_decisions
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_memory_no_truncate BEFORE TRUNCATE ON core_gift.legacy_risk_memories
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_claim_no_truncate BEFORE TRUNCATE ON core_gift.legacy_claims
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_import_no_truncate BEFORE TRUNCATE ON core_gift.legacy_imports
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_gift.legacy_installations,core_gift.legacy_bindings,
 core_gift.legacy_decisions,core_gift.legacy_risk_memories,core_gift.legacy_profiles,
 core_gift.legacy_fingerprints,core_gift.legacy_claims,core_gift.legacy_imports FROM PUBLIC;
REVOKE ALL ON SEQUENCE core_gift.legacy_fingerprints_id_seq FROM PUBLIC;
REVOKE ALL ON FUNCTION core_gift.check_legacy_binding(),core_gift.check_legacy_claim() FROM PUBLIC;
REVOKE ALL ON FUNCTION core_gift.legacy_oauth_subject(bigint,text) FROM PUBLIC;
REVOKE ALL ON FUNCTION core_gift.check_legacy_decision(),core_gift.reject_legacy_native_overlap() FROM PUBLIC;

CREATE TABLE core_meta.legacy_gift_upgrades (
 upgrade_id text PRIMARY KEY,
 request_sha256 bytea NOT NULL CHECK(octet_length(request_sha256)=32),
 request jsonb NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 bytea NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 bytea NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 bytea NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER legacy_gift_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.legacy_gift_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_gift_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.legacy_gift_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.legacy_gift_upgrades FROM PUBLIC;

-- Preserve the original source import guard at this exact assembly.
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
 OR (SELECT count(*) FROM core_meta.component_contract)<>80 THEN
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
