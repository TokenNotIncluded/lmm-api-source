-- Exact legacy invitation links. Attribution only: no referral reward is minted.
CREATE TABLE core_meta.legacy_affiliate_alias_imports (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 import_id TEXT NOT NULL UNIQUE CHECK(import_id ~ '^[A-Za-z0-9._:/-]{1,128}$'),
 material_utf8 TEXT NOT NULL CHECK(material_utf8 IS JSON OBJECT WITH UNIQUE KEYS),
 material JSONB GENERATED ALWAYS AS (material_utf8::jsonb) STORED,
 material_sha256 TEXT NOT NULL CHECK(material_sha256 ~ '^[a-f0-9]{64}$'
  AND material_sha256=encode(sha256(convert_to(material_utf8,'UTF8')),'hex')),
 review_utf8 TEXT NOT NULL CHECK(review_utf8 IS JSON OBJECT WITH UNIQUE KEYS),
 review JSONB GENERATED ALWAYS AS (review_utf8::jsonb) STORED,
 review_sha256 TEXT NOT NULL CHECK(review_sha256 ~ '^[a-f0-9]{64}$'
  AND review_sha256=encode(sha256(convert_to(review_utf8,'UTF8')),'hex')),
 imported_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_identity.legacy_affiliate_aliases (
 code TEXT COLLATE "C" PRIMARY KEY CHECK(code COLLATE "C" ~ '^[A-Za-z0-9]{4}$'),
 user_id BIGINT NOT NULL UNIQUE REFERENCES core_identity.users(id),
 source_user_id BIGINT NOT NULL UNIQUE CHECK(source_user_id>0),
 source_row_sha256 TEXT NOT NULL CHECK(source_row_sha256 ~ '^[a-f0-9]{64}$'),
 import_id TEXT NOT NULL REFERENCES core_meta.legacy_affiliate_alias_imports(import_id)
);
REVOKE ALL ON core_meta.legacy_affiliate_alias_imports,core_identity.legacy_affiliate_aliases FROM PUBLIC;

-- Same concrete protected-authority projection as the frozen c343 identity writer.
-- This helper and the source receipt are never granted to the runtime.
CREATE FUNCTION core_meta.legacy_affiliate_alias_authority(mapping jsonb) RETURNS jsonb
LANGUAGE sql STABLE SET search_path=pg_catalog AS $authority$
 SELECT coalesce(jsonb_agg(jsonb_build_object(
  'source_user_id',mp.source_user_id,'target_user_id',u.id,'target_personal_account_id',u.personal_account_id,
  'active',u.active,'account_active',a.active,'platform_role',u.platform_role,'auth_version',u.auth_version,
  'password_login_allowed',p.password_login_allowed,'second_factor_required',p.second_factor_required,
  'developer_granted',d.granted,'developer_revision',d.revision) ORDER BY mp.source_user_id),'[]'::jsonb)
 FROM jsonb_to_recordset(mapping) AS mp(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)
 JOIN core_identity.users u ON u.id=mp.target_user_id AND u.personal_account_id=mp.target_personal_account_id
 JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.kind='personal'
 JOIN core_identity.user_profiles p ON p.user_id=u.id
 LEFT JOIN core_identity.developer_access d ON d.user_id=u.id
$authority$;
REVOKE ALL ON FUNCTION core_meta.legacy_affiliate_alias_authority(jsonb) FROM PUBLIC;

CREATE FUNCTION core_meta.guard_legacy_affiliate_alias_import() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $guard$
DECLARE
 m jsonb:=NEW.material_utf8::jsonb;
 r jsonb:=NEW.review_utf8::jsonb;
 ir core_migration.import_runs%ROWTYPE;
 mapping jsonb;
 sources jsonb;
 actual_mapping jsonb;
 owner_name name;
 source_count bigint;
 alias_count bigint;
BEGIN
 SELECT pg_get_userbyid(datdba) INTO owner_name FROM pg_database WHERE datname=current_database();
 IF current_user<>owner_name OR NOT EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin)
 THEN RAISE EXCEPTION 'affiliate_alias_owner_role_required'; END IF;
 IF m->>'format' IS DISTINCT FROM 'lmm.current-affiliate-alias-import.v1'
  OR m->>'scope' IS DISTINCT FROM 'current-legacy-affiliate-aliases-v1'
  OR m->>'migration_id' IS DISTINCT FROM NEW.import_id
  OR coalesce(m->>'source_kind' NOT IN ('VERIFIED_FULL_CANONICAL_SOURCE','SYNTHETIC_ALIAS_FIXTURE'),true)
  OR jsonb_typeof(m->'mapping') IS DISTINCT FROM 'array'
  OR jsonb_typeof(m->'source'->'rows') IS DISTINCT FROM 'array'
  OR jsonb_typeof(m->'identity'->'actual_authority') IS DISTINCT FROM 'array'
  OR EXISTS(SELECT FROM unnest(ARRAY['source_archive_sha256','source_export_sha256',
     'source_export_proof_sha256','source_users_sha256','source_codes_sha256']) f
     WHERE coalesce(m->'source'->>f !~ '^[a-f0-9]{64}$',true))
 THEN RAISE EXCEPTION 'affiliate_alias_material_invalid'; END IF;
 IF m->>'source_kind'='SYNTHETIC_ALIAS_FIXTURE' THEN
  IF current_database() !~ '^lmm_runtime_keys_test_[a-z0-9_]{1,30}$'
   OR coalesce(m->>'fixture_nonce' !~ '^[a-f0-9]{64}$',true)
   OR shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database')
      IS DISTINCT FROM 'lmm-runtime-keys-synthetic:'||(m->>'fixture_nonce')
   OR r IS DISTINCT FROM jsonb_build_object('scope','synthetic-alias-test-only',
       'material_sha256',NEW.material_sha256,'production_authorization',false)
  THEN RAISE EXCEPTION 'affiliate_alias_synthetic_target_required'; END IF;
 ELSE
  IF r->>'scope' IS DISTINCT FROM 'reviewed-current-legacy-affiliate-aliases-v1'
   OR r->>'material_sha256' IS DISTINCT FROM NEW.material_sha256
   OR r->'migration_actor_user_id' IS DISTINCT FROM m->'migration_actor_user_id'
   OR r->'source_fence_sha256' IS DISTINCT FROM m->'source_fence_sha256'
   OR r->'target' IS DISTINCT FROM m->'target'
   OR coalesce(m->>'source_fence_sha256' !~ '^[a-f0-9]{64}$',true)
   OR m->'target'->>'component_count' IS DISTINCT FROM '86'
   OR m->'target'->>'database' IS DISTINCT FROM current_database()
   OR m->'target'->>'database_oid' IS DISTINCT FROM
       (SELECT oid::text FROM pg_database WHERE datname=current_database())
   OR m->'target'->>'system_identifier' IS DISTINCT FROM
       (SELECT system_identifier::text FROM pg_control_system())
   OR m->'target'->>'server_instance_nonce' IS DISTINCT FROM current_setting('lmm.migration_instance_nonce',true)
  THEN RAISE EXCEPTION 'affiliate_alias_review_or_target_binding_invalid'; END IF;
 END IF;
 IF NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
  WHERE u.id=(m->>'migration_actor_user_id')::bigint AND u.active AND a.active AND u.platform_role='superadmin')
 THEN RAISE EXCEPTION 'affiliate_alias_enabled_migration_actor_required'; END IF;
 SELECT * INTO ir FROM core_migration.import_runs WHERE migration_id=m->'identity'->>'migration_id';
 IF NOT FOUND OR ir.manifest->>'scope' IS DISTINCT FROM 'current-identity-append-completion-v1'
  OR ir.source_archive_sha256 IS DISTINCT FROM m->'source'->>'source_archive_sha256'
  OR ir.mapping_sha256 IS DISTINCT FROM m->'identity'->>'mapping_sha256'
  OR encode(sha256(convert_to(ir.manifest::text,'UTF8')),'hex') IS DISTINCT FROM m->'identity'->>'manifest_sha256'
  OR ir.manifest->>'source_namespace_sha256' IS DISTINCT FROM m->>'source_namespace_sha256'
  OR ir.manifest->'material'->>'source_canonical_export_sha256' IS DISTINCT FROM m->'source'->>'source_export_sha256'
  OR ir.manifest->'material'->>'source_canonical_export_proof_sha256' IS DISTINCT FROM m->'source'->>'source_export_proof_sha256'
  OR ir.manifest->'mapping' IS DISTINCT FROM ir.manifest->'material'->'plan'->'mapping'
  OR ir.manifest->'auth_version_lineage' IS DISTINCT FROM ir.manifest->'material'->'plan'->'auth_version_lineage'
  OR ir.manifest->'actual_identity_authority' IS DISTINCT FROM m->'identity'->'actual_authority'
 THEN RAISE EXCEPTION 'affiliate_alias_actual_identity_receipt_required'; END IF;
 mapping:=m->'mapping';sources:=m->'source'->'rows';
 source_count:=jsonb_array_length(sources);
 alias_count:=(SELECT count(*) FROM jsonb_array_elements(sources) s WHERE s->>'code' IS NOT NULL);
 IF source_count<1 OR source_count IS DISTINCT FROM (m->'source'->>'user_count')::bigint
  OR alias_count IS DISTINCT FROM (m->'source'->>'alias_count')::bigint
  OR jsonb_array_length(mapping)<>source_count
  OR (SELECT count(DISTINCT (x->>'source_user_id')::bigint) FROM jsonb_array_elements(mapping) x)<>source_count
  OR (SELECT count(DISTINCT (x->>'target_user_id')::bigint) FROM jsonb_array_elements(mapping) x)<>source_count
  OR (SELECT count(DISTINCT (x->>'target_personal_account_id')::bigint) FROM jsonb_array_elements(mapping) x)<>source_count
  OR (SELECT count(DISTINCT (x->>'source_user_id')::bigint) FROM jsonb_array_elements(sources) x)<>source_count
  OR (SELECT count(DISTINCT (x->>'code') COLLATE "C") FROM jsonb_array_elements(sources) x)<>alias_count
  OR EXISTS(SELECT FROM jsonb_array_elements(sources) x WHERE NOT (x ? 'code')
    OR coalesce(x->>'source_row_sha256' !~ '^[a-f0-9]{64}$',true)
    OR coalesce((x->>'source_user_id')::bigint<=0,true)
    OR (x->>'code' IS NOT NULL AND (x->>'code') COLLATE "C" !~ '^[A-Za-z0-9]{4}$'))
 THEN RAISE EXCEPTION 'affiliate_alias_source_coverage_invalid'; END IF;
 SELECT jsonb_agg(jsonb_build_object('source_user_id',(x->>'source_user_id')::bigint,
  'target_user_id',(x->>'target_user_id')::bigint,
  'target_personal_account_id',(x->>'target_personal_account_id')::bigint)
  ORDER BY (x->>'source_user_id')::bigint)
 INTO actual_mapping FROM jsonb_array_elements(ir.manifest->'mapping') x;
 IF actual_mapping IS DISTINCT FROM mapping
  OR EXISTS(SELECT FROM jsonb_array_elements(sources) s
   LEFT JOIN jsonb_array_elements(ir.manifest->'auth_version_lineage') l ON l->>'source_user_id'=s->>'source_user_id'
   WHERE l IS NULL OR l->>'source_canonical_row_sha256' IS DISTINCT FROM s->>'source_row_sha256')
  OR (SELECT jsonb_agg((s->>'source_user_id')::bigint ORDER BY (s->>'source_user_id')::bigint)
      FROM jsonb_array_elements(sources) s)
     IS DISTINCT FROM
     (SELECT jsonb_agg((x->>'source_user_id')::bigint ORDER BY (x->>'source_user_id')::bigint)
      FROM jsonb_array_elements(mapping) x)
 THEN RAISE EXCEPTION 'affiliate_alias_full_identity_map_invalid'; END IF;
 IF core_meta.legacy_affiliate_alias_authority(mapping) IS DISTINCT FROM m->'identity'->'actual_authority'
 THEN RAISE EXCEPTION 'affiliate_alias_current_identity_authority_changed'; END IF;
 RETURN NEW;
END $guard$;
REVOKE ALL ON FUNCTION core_meta.guard_legacy_affiliate_alias_import() FROM PUBLIC;
CREATE TRIGGER legacy_affiliate_alias_import_guard BEFORE INSERT ON core_meta.legacy_affiliate_alias_imports
 FOR EACH ROW EXECUTE FUNCTION core_meta.guard_legacy_affiliate_alias_import();

CREATE FUNCTION core_meta.guard_legacy_affiliate_alias_row() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $guard$
DECLARE m jsonb;
BEGIN
 SELECT material INTO m FROM core_meta.legacy_affiliate_alias_imports WHERE import_id=NEW.import_id;
 IF NOT FOUND OR NOT EXISTS(
  SELECT FROM jsonb_array_elements(m->'source'->'rows') s
  JOIN jsonb_array_elements(m->'mapping') mp ON mp->>'source_user_id'=s->>'source_user_id'
  WHERE (s->>'source_user_id')::bigint=NEW.source_user_id
    AND (s->>'code') COLLATE "C"=NEW.code AND s->>'source_row_sha256'=NEW.source_row_sha256
    AND (mp->>'target_user_id')::bigint=NEW.user_id)
 THEN RAISE EXCEPTION 'affiliate_alias_row_not_in_reviewed_source'; END IF;
 RETURN NEW;
END $guard$;
REVOKE ALL ON FUNCTION core_meta.guard_legacy_affiliate_alias_row() FROM PUBLIC;
CREATE TRIGGER legacy_affiliate_alias_row_guard BEFORE INSERT ON core_identity.legacy_affiliate_aliases
 FOR EACH ROW EXECUTE FUNCTION core_meta.guard_legacy_affiliate_alias_row();

CREATE FUNCTION core_meta.complete_legacy_affiliate_alias_import() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $guard$
DECLARE actual jsonb;expected jsonb;
BEGIN
 SELECT coalesce(jsonb_agg(jsonb_build_object('code',a.code,'user_id',a.user_id,
  'source_user_id',a.source_user_id,'source_row_sha256',a.source_row_sha256)
  ORDER BY a.source_user_id),'[]'::jsonb) INTO actual
 FROM core_identity.legacy_affiliate_aliases a WHERE a.import_id=NEW.import_id;
 SELECT coalesce(jsonb_agg(jsonb_build_object('code',s->>'code','user_id',(mp->>'target_user_id')::bigint,
  'source_user_id',(s->>'source_user_id')::bigint,'source_row_sha256',s->>'source_row_sha256')
  ORDER BY (s->>'source_user_id')::bigint),'[]'::jsonb) INTO expected
 FROM jsonb_array_elements(NEW.material->'source'->'rows') s
 JOIN jsonb_array_elements(NEW.material->'mapping') mp ON mp->>'source_user_id'=s->>'source_user_id'
 WHERE s->>'code' IS NOT NULL;
 IF actual IS DISTINCT FROM expected THEN RAISE EXCEPTION 'affiliate_alias_import_not_complete'; END IF;
 IF core_meta.legacy_affiliate_alias_authority(NEW.material->'mapping')
      IS DISTINCT FROM NEW.material->'identity'->'actual_authority'
  OR NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
    WHERE u.id=(NEW.material->>'migration_actor_user_id')::bigint AND u.active AND a.active AND u.platform_role='superadmin')
 THEN RAISE EXCEPTION 'affiliate_alias_commit_authority_changed'; END IF;
 RETURN NULL;
END $guard$;
REVOKE ALL ON FUNCTION core_meta.complete_legacy_affiliate_alias_import() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER legacy_affiliate_alias_import_complete AFTER INSERT ON core_meta.legacy_affiliate_alias_imports
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_meta.complete_legacy_affiliate_alias_import();

CREATE TRIGGER legacy_affiliate_aliases_immutable BEFORE UPDATE OR DELETE ON core_identity.legacy_affiliate_aliases
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_affiliate_aliases_no_truncate BEFORE TRUNCATE ON core_identity.legacy_affiliate_aliases
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_affiliate_alias_imports_immutable BEFORE UPDATE OR DELETE ON core_meta.legacy_affiliate_alias_imports
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_affiliate_alias_imports_no_truncate BEFORE TRUNCATE ON core_meta.legacy_affiliate_alias_imports
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();

ALTER TABLE core_identity.affiliate_registrations DROP CONSTRAINT affiliate_registrations_inviter_id_fkey;
ALTER TABLE core_identity.affiliate_registrations ADD CONSTRAINT affiliate_registrations_inviter_id_fkey
 FOREIGN KEY(inviter_id) REFERENCES core_identity.users(id);
CREATE FUNCTION core_identity.guard_affiliate_registration_inviter() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $guard$
DECLARE enabled boolean;
BEGIN
 SELECT u.active AND a.active AND a.kind='personal' INTO enabled
 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
 WHERE u.id=NEW.inviter_id FOR SHARE OF u,a;
 IF enabled IS DISTINCT FROM TRUE OR NOT (
   EXISTS(SELECT FROM core_identity.affiliate_codes c WHERE c.user_id=NEW.inviter_id)
   OR EXISTS(SELECT FROM core_identity.legacy_affiliate_aliases a
      JOIN core_meta.legacy_affiliate_alias_imports r ON r.import_id=a.import_id WHERE a.user_id=NEW.inviter_id))
 THEN RAISE EXCEPTION 'affiliate_registration_inviter_unavailable'; END IF;
 RETURN NEW;
END $guard$;
REVOKE ALL ON FUNCTION core_identity.guard_affiliate_registration_inviter() FROM PUBLIC;
CREATE TRIGGER affiliate_registration_inviter_guard BEFORE INSERT ON core_identity.affiliate_registrations
 FOR EACH ROW EXECUTE FUNCTION core_identity.guard_affiliate_registration_inviter();

CREATE TABLE core_meta.legacy_affiliate_alias_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER legacy_affiliate_alias_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.legacy_affiliate_alias_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_affiliate_alias_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.legacy_affiliate_alias_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.legacy_affiliate_alias_upgrades FROM PUBLIC;
