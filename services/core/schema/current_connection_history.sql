-- Current, source-bound acquisition history on an already-used target.
-- Historical connection_snapshot, its guard and the 55/56/77 importer stay frozen.
-- The root integration binds the final suffix number before catalog capture.
CREATE TABLE core_history.current_connection_snapshot (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 migration_id TEXT NOT NULL UNIQUE CHECK(length(migration_id) BETWEEN 1 AND 128),
 source_namespace_sha256 TEXT NOT NULL CHECK(source_namespace_sha256 ~ '^[a-f0-9]{64}$'),
 source_archive_sha256 TEXT NOT NULL CHECK(source_archive_sha256 ~ '^[a-f0-9]{64}$'),
 export_file_sha256 TEXT NOT NULL CHECK(export_file_sha256 ~ '^[a-f0-9]{64}$'),
 document_utf8 TEXT NOT NULL,
 document JSONB GENERATED ALWAYS AS (document_utf8::jsonb) STORED,
 metadata JSONB NOT NULL CHECK(jsonb_typeof(metadata)='object'),
 mapping_utf8 TEXT NOT NULL,
 mapping JSONB GENERATED ALWAYS AS (mapping_utf8::jsonb) STORED,
 mapping_sha256 TEXT NOT NULL CHECK(mapping_sha256 ~ '^[a-f0-9]{64}$'),
 identity_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id),
 identity_mapping_sha256 TEXT NOT NULL CHECK(identity_mapping_sha256 ~ '^[a-f0-9]{64}$'),
 key_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id),
 key_owner_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id),
 key_mapping_sha256 TEXT NOT NULL CHECK(key_mapping_sha256 ~ '^[a-f0-9]{64}$'),
 key_material_sha256 TEXT NOT NULL CHECK(key_material_sha256 ~ '^[a-f0-9]{64}$'),
 snapshot_observed_at BIGINT NOT NULL CHECK(snapshot_observed_at BETWEEN 1 AND 253402300799),
 target_database TEXT NOT NULL,
 target_database_oid OID NOT NULL,
 target_system_identifier TEXT NOT NULL CHECK(target_system_identifier ~ '^[0-9]+$'),
 target_nonce TEXT NOT NULL CHECK(target_nonce ~ '^[a-f0-9]{64}$' AND target_nonce<>repeat('0',64)),
 catalog_sha256 TEXT NOT NULL CHECK(catalog_sha256 ~ '^[a-f0-9]{64}$'),
 -- Audit of the observed state only. Current runtime classification uses the
 -- explicit credential map, including native keys created before this row.
 native_credential_floor BIGINT NOT NULL CHECK(native_credential_floor>=0),
 migration_actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 imported_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK(export_file_sha256=encode(sha256(convert_to(document_utf8,'UTF8')),'hex')),
 CHECK(mapping_sha256=encode(sha256(convert_to(mapping_utf8,'UTF8')),'hex'))
);

CREATE TABLE core_history.current_connection_credential_map (
 snapshot_singleton BOOLEAN NOT NULL DEFAULT TRUE CHECK(snapshot_singleton)
  REFERENCES core_history.current_connection_snapshot(singleton) DEFERRABLE INITIALLY DEFERRED,
 source_key_id BIGINT PRIMARY KEY CHECK(source_key_id>0),
 target_credential_id BIGINT NOT NULL UNIQUE REFERENCES core_identity.credentials(id),
 user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 identity_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id),
 key_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id),
 key_owner_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id)
);

CREATE FUNCTION core_history.guard_current_connection_map() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF current_user<>pg_get_userbyid((SELECT c.relowner FROM pg_class c
   WHERE c.oid='core_history.current_connection_credential_map'::regclass))
 OR current_setting('transaction_isolation')<>'serializable'
 OR current_setting('transaction_read_only')<>'off'
 OR (SELECT r.rolcanlogin FROM pg_roles r WHERE r.rolname=current_user) THEN
  RAISE EXCEPTION 'current connection map requires serializable offline owner' USING ERRCODE='42501';
 END IF;
 IF EXISTS(SELECT FROM core_history.connection_snapshot)
 OR EXISTS(SELECT FROM core_history.current_connection_snapshot) THEN
  RAISE EXCEPTION 'current connection mapping cannot extend a committed snapshot';
 END IF;
 IF NOT EXISTS(SELECT FROM core_identity.credentials c JOIN core_identity.users u ON u.id=c.user_id
  JOIN core_identity.accounts a ON a.id=c.owner_account_id
  WHERE c.id=NEW.target_credential_id AND c.kind='api_key'
  AND c.user_id=NEW.user_id AND c.owner_account_id=NEW.account_id
  AND u.personal_account_id=a.id AND a.kind='personal') THEN
  RAISE EXCEPTION 'current connection credential owner mismatch';
 END IF;
 -- Full receipts and the entire source/native set are checked exactly once by
 -- the snapshot guard after all map rows. The deferred FK prevents a map-only
 -- transaction from committing; a snapshot cannot be inserted before its map.
 RETURN NEW;
END $$;

CREATE FUNCTION core_history.guard_current_connection_snapshot() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE d JSONB:=NEW.document_utf8::jsonb; m JSONB:=NEW.mapping_utf8::jsonb;
 identity_run core_migration.import_runs%ROWTYPE; key_run core_migration.import_runs%ROWTYPE;
 owner_run core_migration.import_runs%ROWTYPE; im JSONB; km JSONB; om JSONB;
 table_name_ TEXT; nonce_ TEXT;
BEGIN
 IF current_user<>pg_get_userbyid((SELECT c.relowner FROM pg_class c
   WHERE c.oid='core_history.current_connection_snapshot'::regclass))
 OR current_setting('transaction_isolation')<>'serializable'
 OR current_setting('transaction_read_only')<>'off'
 OR (SELECT r.rolcanlogin FROM pg_roles r WHERE r.rolname=current_user) THEN
  RAISE EXCEPTION 'current connection import requires serializable offline owner' USING ERRCODE='42501';
 END IF;
 IF EXISTS(SELECT FROM core_history.connection_snapshot) THEN
  RAISE EXCEPTION 'current connection legacy snapshot supersession is not supported';
 END IF;
 IF NEW.target_database<>current_database()
 OR NEW.target_database_oid<>(SELECT p.oid FROM pg_database p WHERE p.datname=current_database())
 OR NEW.target_system_identifier<>(SELECT p.system_identifier::text FROM pg_control_system() p)
 OR NEW.catalog_sha256<>(SELECT encode(r.catalog_fingerprint,'hex') FROM core_meta.runtime_contract r WHERE r.singleton)
 OR (SELECT count(*) FROM core_meta.component_contract)<>84 THEN
  RAISE EXCEPTION 'current connection target contract mismatch';
 END IF;
 SELECT substring(c FROM length('lmm.migration_instance_nonce=')+1) INTO STRICT nonce_
 FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) c
 WHERE s.setdatabase=NEW.target_database_oid AND s.setrole=0 AND c LIKE 'lmm.migration_instance_nonce=%';
 IF nonce_ IS DISTINCT FROM NEW.target_nonce THEN RAISE EXCEPTION 'current connection target nonce mismatch'; END IF;
 IF EXISTS(SELECT FROM pg_stat_activity a WHERE a.datid=NEW.target_database_oid AND a.pid<>pg_backend_pid())
 OR EXISTS(SELECT FROM pg_prepared_xacts p WHERE p.database=current_database()) THEN
  RAISE EXCEPTION 'current connection target is not offline';
 END IF;
 IF NEW.native_credential_floor<>(SELECT coalesce(max(c.id),0) FROM core_identity.credentials c) THEN
  RAISE EXCEPTION 'current connection observed credential maximum changed';
 END IF;
 IF NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
  WHERE u.id=NEW.migration_actor_user_id AND u.active AND a.active AND u.platform_role='superadmin') THEN
  RAISE EXCEPTION 'current connection actual migration actor missing';
 END IF;

 SELECT * INTO STRICT identity_run FROM core_migration.import_runs r WHERE r.migration_id=NEW.identity_migration_id;
 SELECT * INTO STRICT key_run FROM core_migration.import_runs r WHERE r.migration_id=NEW.key_migration_id;
 SELECT * INTO STRICT owner_run FROM core_migration.import_runs r WHERE r.migration_id=NEW.key_owner_migration_id;
 im:=identity_run.manifest->'material'; km:=key_run.manifest->'material'; om:=owner_run.manifest->'material';
 IF NOT coalesce(identity_run.manifest->>'scope'='current-identity-append-completion-v1'
 AND im->>'scope'=identity_run.manifest->>'scope'
 AND identity_run.source_archive_sha256=NEW.source_archive_sha256
 AND im->>'source_archive_sha256'=NEW.source_archive_sha256
 AND identity_run.mapping_sha256=NEW.identity_mapping_sha256
 AND im->>'mapping_sha256'=NEW.identity_mapping_sha256
 AND identity_run.manifest->>'source_namespace_sha256'=NEW.source_namespace_sha256
 AND im->>'source_namespace_sha256'=NEW.source_namespace_sha256
 AND identity_run.manifest->'mapping'=im->'plan'->'mapping'
 AND identity_run.manifest->'auth_version_lineage'=im->'plan'->'auth_version_lineage'
 AND identity_run.manifest->'actual_identity_authority'=im->'plan'->'expected_identity_authority',FALSE) THEN
  RAISE EXCEPTION 'current connection committed identity receipt mismatch';
 END IF;
 IF NOT coalesce(key_run.manifest->>'scope'='current-core81-runtime-key-append-v1'
 AND km->>'scope'=key_run.manifest->>'scope'
 AND key_run.source_archive_sha256=NEW.source_archive_sha256
 AND km->>'source_archive_sha256'=NEW.source_archive_sha256
 AND key_run.mapping_sha256=NEW.key_mapping_sha256 AND km->>'mapping_sha256'=NEW.key_mapping_sha256
 AND key_run.manifest->>'source_namespace_sha256'=NEW.source_namespace_sha256
 AND km->>'source_namespace_sha256'=NEW.source_namespace_sha256
 AND key_run.manifest->'mapping'=km->'credential_receipt'->'records'
 AND km->'tool_source_hashes'->>'current_runtime_keys81.py'='51313c45f0f6fc9f614b8f70deb6d3540d2d9625cca16f600029a7604e81a7e0'
 AND km->'tool_source_hashes'->>'current_key_lineage.py'='0835d57cb70a58ed8bdf368d5c25d588aca0139dfdb44389e2550981953c9da7'
 AND km->'tool_source_hashes'->>'migrate_current_identity.py'='c3430d1d03f201dcf254bb31678e7af281d94136cf64b6e661881907f6b3b376'
 AND owner_run.migration_id<>key_run.migration_id
 AND owner_run.manifest->>'scope'='current-core81-rights-append-v1'
 AND om->>'scope'=owner_run.manifest->>'scope'
 AND owner_run.source_archive_sha256=NEW.source_archive_sha256
 AND om->>'source_archive_sha256'=NEW.source_archive_sha256
 AND owner_run.mapping_sha256=NEW.key_mapping_sha256 AND om->>'mapping_sha256'=NEW.key_mapping_sha256
 AND owner_run.manifest->>'component_scope'=km->>'scope'
 AND owner_run.manifest->>'component_material_sha256'=NEW.key_material_sha256
 AND om->'component_material'=km,FALSE) THEN
  RAISE EXCEPTION 'current connection committed key and owner receipt mismatch';
 END IF;

 IF NOT coalesce(d->>'contract'='current.connection-source.v1'
 AND m->>'contract'='current.connection-mapping.v1' AND m->'omitted_users'='[]'::jsonb
 AND d->>'coverage'='all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked'
 AND NEW.metadata->>'contract'='current.connection-export-binding.v1'
 AND NEW.metadata->>'source_archive_sha256'=NEW.source_archive_sha256
 AND NEW.metadata->>'source_schema'=d->>'source_schema'
 AND NEW.metadata->>'export_file_sha256'=NEW.export_file_sha256
 AND NEW.metadata->>'coverage'=d->>'coverage'
 AND (NEW.metadata->>'snapshot_observed_at')::bigint=NEW.snapshot_observed_at
 AND NEW.snapshot_observed_at<=(d->>'observed_at')::bigint
 AND NEW.metadata->>'source_canonical_export_sha256'=im->>'source_canonical_export_sha256'
 AND NEW.metadata->>'source_canonical_export_proof_sha256'=im->>'source_canonical_export_proof_sha256'
 AND NEW.metadata->>'source_canonical_receipt_sha256'=im->>'source_canonical_receipt_sha256'
 AND NEW.metadata->>'ordinary_key_source_sha256'=km->>'source_input_sha256',FALSE) THEN
  RAISE EXCEPTION 'current connection complete source binding mismatch';
 END IF;
 FOREACH table_name_ IN ARRAY ARRAY['users','tokens','oauth_server_grants','developer_access_requests'] LOOP
  IF jsonb_typeof(d->table_name_) IS DISTINCT FROM 'array'
  OR (d->'counts'->>table_name_)::bigint IS DISTINCT FROM jsonb_array_length(d->table_name_)::bigint
  OR EXISTS(SELECT FROM jsonb_array_elements(d->table_name_) AS e(value)
    GROUP BY e.value->>'id' HAVING count(*)<>1) THEN
   RAISE EXCEPTION 'current connection source array coverage mismatch';
  END IF;
 END LOOP;
 IF jsonb_typeof(m->'users') IS DISTINCT FROM 'array'
 OR jsonb_array_length(m->'users')<>jsonb_array_length(d->'users')
 OR jsonb_array_length(m->'users')<>jsonb_array_length(im->'plan'->'mapping')
 OR EXISTS(SELECT FROM jsonb_array_elements(m->'users') AS e(value) GROUP BY e.value->>'source_user_id' HAVING count(*)<>1)
 OR EXISTS(SELECT FROM jsonb_array_elements(m->'users') AS e(value) GROUP BY e.value->>'user_id' HAVING count(*)<>1)
 OR EXISTS(SELECT FROM jsonb_array_elements(m->'users') AS e(value) GROUP BY e.value->>'account_id' HAVING count(*)<>1)
 OR EXISTS(WITH
 source_users AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(d->'users')
  AS s(id bigint,created_at bigint,source_canonical_row_sha256 text)),
 current_map AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(m->'users')
  AS s(source_user_id bigint,user_id bigint,account_id bigint)),
 identity_map AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(im->'plan'->'mapping')
  AS s(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)),
 lineage AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(im->'plan'->'auth_version_lineage')
  AS s(source_user_id bigint,source_canonical_row_sha256 text))
 SELECT FROM source_users su LEFT JOIN current_map cm ON cm.source_user_id=su.id
 LEFT JOIN identity_map ip ON ip.source_user_id=su.id LEFT JOIN lineage li ON li.source_user_id=su.id
 LEFT JOIN core_identity.users u ON u.id=cm.user_id
 LEFT JOIN core_identity.accounts a ON a.id=cm.account_id
 WHERE cm.source_user_id IS NULL OR ip.source_user_id IS NULL OR li.source_user_id IS NULL
  OR u.id IS NULL OR a.id IS NULL OR ip.target_user_id IS DISTINCT FROM cm.user_id
  OR ip.target_personal_account_id IS DISTINCT FROM cm.account_id
  OR li.source_canonical_row_sha256 IS DISTINCT FROM su.source_canonical_row_sha256
  OR u.personal_account_id IS DISTINCT FROM a.id OR a.kind IS DISTINCT FROM 'personal'
  OR (coalesce(su.created_at,0)>0 AND floor(extract(epoch FROM a.created_at))::bigint IS DISTINCT FROM su.created_at)) THEN
  RAISE EXCEPTION 'current connection complete current owner mapping mismatch';
 END IF;
 IF EXISTS(SELECT FROM jsonb_array_elements(identity_run.manifest->'actual_identity_authority') AS ia(value)
  WHERE NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
   JOIN core_identity.user_profiles p ON p.user_id=u.id LEFT JOIN core_identity.developer_access g ON g.user_id=u.id
   WHERE u.id=(ia.value->>'target_user_id')::bigint AND a.id=(ia.value->>'target_personal_account_id')::bigint
   AND jsonb_build_object('active',u.active,'account_active',a.active,'platform_role',u.platform_role,'auth_version',u.auth_version,
    'password_login_allowed',p.password_login_allowed,'second_factor_required',p.second_factor_required,
    'developer_granted',g.granted,'developer_revision',g.revision)
    =ia.value-ARRAY['source_user_id','target_user_id','target_personal_account_id'])) THEN
  RAISE EXCEPTION 'current connection committed identity authority changed';
 END IF;

 IF EXISTS(SELECT FROM jsonb_array_elements(d->'tokens') AS t(value)
  WHERE jsonb_typeof(t.value->'oauth_managed') IS DISTINCT FROM 'boolean'
   OR jsonb_typeof(t.value->'creation_source') IS DISTINCT FROM 'string'
   OR coalesce(t.value->>'source_row_sha256','') !~ '^[a-f0-9]{64}$')
 OR (SELECT count(*) FROM core_history.current_connection_credential_map)
    <>jsonb_array_length(key_run.manifest->'mapping')
 OR jsonb_array_length(km->'records')<>jsonb_array_length(key_run.manifest->'mapping')
 OR (SELECT count(*) FROM jsonb_array_elements(d->'tokens') AS t(value)
     WHERE (t.value->>'oauth_managed')::boolean=FALSE AND t.value->>'creation_source'<>'assistant_runtime')
    <>jsonb_array_length(key_run.manifest->'mapping') THEN
  RAISE EXCEPTION 'current connection complete ordinary key coverage mismatch';
 END IF;
 IF EXISTS(WITH
 key_map AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(key_run.manifest->'mapping') AS s(
  source_key_id bigint,target_credential_id bigint,target_user_id bigint,target_account_id bigint,source_secret_sha256 text)),
 key_rows AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(km->'records') AS s(
  source_token_id bigint,target_credential_id bigint,target_user_id bigint,target_personal_account_id bigint,
  digest_sha256 text,target_auth_version bigint,created_time bigint,source_row_sha256 text,source_deleted_at jsonb)),
 source_tokens AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(d->'tokens') AS s(
  id bigint,user_id bigint,created_time bigint,source_row_sha256 text,deleted_at jsonb,oauth_managed boolean,creation_source text)),
 user_map AS MATERIALIZED (SELECT * FROM jsonb_to_recordset(m->'users') AS s(source_user_id bigint,user_id bigint,account_id bigint))
 SELECT FROM core_history.current_connection_credential_map cm
 LEFT JOIN key_map kr ON kr.source_key_id=cm.source_key_id
 LEFT JOIN key_rows row_ ON row_.source_token_id=cm.source_key_id
 LEFT JOIN source_tokens st ON st.id=cm.source_key_id LEFT JOIN user_map mu ON mu.source_user_id=st.user_id
 LEFT JOIN core_identity.credentials c ON c.id=cm.target_credential_id
 WHERE
  cm.identity_migration_id<>NEW.identity_migration_id OR cm.key_migration_id<>NEW.key_migration_id
  OR cm.key_owner_migration_id<>NEW.key_owner_migration_id
  OR kr.source_key_id IS NULL OR row_.source_token_id IS NULL OR st.id IS NULL OR mu.source_user_id IS NULL OR c.id IS NULL
  OR kr.target_credential_id IS DISTINCT FROM cm.target_credential_id
  OR cm.user_id IS DISTINCT FROM c.user_id OR cm.account_id IS DISTINCT FROM c.owner_account_id
  OR kr.target_user_id IS DISTINCT FROM cm.user_id OR kr.target_account_id IS DISTINCT FROM cm.account_id
  OR mu.user_id IS DISTINCT FROM cm.user_id OR mu.account_id IS DISTINCT FROM cm.account_id
  OR row_.target_credential_id IS DISTINCT FROM kr.target_credential_id
  OR row_.target_user_id IS DISTINCT FROM kr.target_user_id
  OR row_.target_personal_account_id IS DISTINCT FROM kr.target_account_id
  OR c.kind IS DISTINCT FROM 'api_key' OR encode(c.digest,'hex') IS DISTINCT FROM kr.source_secret_sha256
  OR row_.digest_sha256 IS DISTINCT FROM kr.source_secret_sha256
  OR c.user_version IS DISTINCT FROM row_.target_auth_version
  OR c.created_at IS DISTINCT FROM to_timestamp(row_.created_time)
  OR st.created_time IS DISTINCT FROM row_.created_time
  OR st.source_row_sha256 IS DISTINCT FROM row_.source_row_sha256
  OR st.deleted_at IS DISTINCT FROM row_.source_deleted_at
  OR st.oauth_managed IS DISTINCT FROM FALSE OR st.creation_source='assistant_runtime') THEN
  RAISE EXCEPTION 'current connection actual imported credential mapping mismatch';
 END IF;
 RETURN NEW;
END $$;

CREATE TRIGGER current_connection_map_bound BEFORE INSERT ON core_history.current_connection_credential_map
 FOR EACH ROW EXECUTE FUNCTION core_history.guard_current_connection_map();
CREATE TRIGGER current_connection_snapshot_bound BEFORE INSERT ON core_history.current_connection_snapshot
 FOR EACH ROW EXECUTE FUNCTION core_history.guard_current_connection_snapshot();
CREATE TRIGGER current_connection_map_immutable BEFORE UPDATE OR DELETE ON core_history.current_connection_credential_map
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER current_connection_map_no_truncate BEFORE TRUNCATE ON core_history.current_connection_credential_map
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER current_connection_snapshot_immutable BEFORE UPDATE OR DELETE ON core_history.current_connection_snapshot
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER current_connection_snapshot_no_truncate BEFORE TRUNCATE ON core_history.current_connection_snapshot
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_history.current_connection_snapshot,core_history.current_connection_credential_map FROM PUBLIC;
REVOKE ALL ON FUNCTION core_history.guard_current_connection_snapshot(),core_history.guard_current_connection_map() FROM PUBLIC;

-- Exact offline predecessor -> current-history suffix receipt. Runtime receives
-- no privileges on upgrade receipts; it reads only the two acquisition tables.
CREATE TABLE core_meta.current_connection_history_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER current_connection_history_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.current_connection_history_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER current_connection_history_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.current_connection_history_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.current_connection_history_upgrades FROM PUBLIC;
