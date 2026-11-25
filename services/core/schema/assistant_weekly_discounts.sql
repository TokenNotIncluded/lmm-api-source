-- Component 88: durable weekly decisions and private single-use coupons.
-- Trust levels are retired. The owner explicitly activates the ordinary-user policy.
CREATE SCHEMA core_weekly;
REVOKE ALL ON SCHEMA core_weekly FROM PUBLIC;

CREATE TABLE core_weekly.weekly_discount_policy (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    maximum_percent SMALLINT NOT NULL DEFAULT 10 CHECK(maximum_percent BETWEEN 1 AND 99)
);
INSERT INTO core_weekly.weekly_discount_policy(singleton) VALUES(TRUE);

CREATE TABLE core_weekly.weekly_discounts (
    id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    week_start BIGINT NOT NULL CHECK(week_start>=345600 AND (week_start-345600)%604800=0),
    origin TEXT NOT NULL DEFAULT 'native' CHECK(origin IN ('native','legacy')),
    status TEXT NOT NULL CHECK(status IN ('declined','offered','claimed')),
    discount_percent SMALLINT NOT NULL CHECK(discount_percent BETWEEN 0 AND 99),
    reason TEXT NOT NULL CHECK(char_length(reason) BETWEEN 2 AND 240),
    conversation_id TEXT REFERENCES core_assistant.conversations(id),
    operation_id TEXT UNIQUE REFERENCES core_assistant.agent_tool_operations(id),
    credential_id BIGINT REFERENCES core_identity.credentials(id),
    auth_version BIGINT CHECK(auth_version>=0),
    web_session_sid TEXT REFERENCES core_identity.web_sessions(sid),
    created_at BIGINT NOT NULL CHECK(created_at>=0),
    code_id BIGINT UNIQUE REFERENCES core_payments.discount_codes(id),
    claimed_at BIGINT,
    claim_nonce_sha256 BYTEA CHECK(claim_nonce_sha256 IS NULL OR octet_length(claim_nonce_sha256)=32),
    claim_nonce_expires_at BIGINT,
    UNIQUE(user_id,week_start),
    CHECK((status='declined' AND discount_percent=0 AND code_id IS NULL AND claimed_at IS NULL)
       OR (status='offered' AND discount_percent>0 AND code_id IS NULL AND claimed_at IS NULL)
       OR (status='claimed' AND discount_percent>0 AND code_id IS NOT NULL AND claimed_at IS NOT NULL AND claimed_at>=created_at)),
    CHECK((origin='native' AND num_nonnulls(conversation_id,operation_id,credential_id,auth_version,web_session_sid)=5
              AND created_at>=week_start AND created_at<week_start+604800)
       OR (origin='legacy' AND num_nonnulls(conversation_id,operation_id,credential_id,auth_version,web_session_sid)=0)),
    CHECK(num_nonnulls(claim_nonce_sha256,claim_nonce_expires_at) IN (0,2))
);
-- Source evidence belongs to the offline owner, not the running assistant.
CREATE TABLE core_meta.legacy_weekly_discount_imports (
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

CREATE TABLE core_weekly.legacy_weekly_discount_bases (
    decision_id TEXT PRIMARY KEY REFERENCES core_weekly.weekly_discounts(id)
        DEFERRABLE INITIALLY DEFERRED,
    import_id TEXT NOT NULL REFERENCES core_meta.legacy_weekly_discount_imports(import_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_namespace_sha256 BYTEA NOT NULL CHECK(octet_length(source_namespace_sha256)=32),
    source_archive_sha256 BYTEA NOT NULL CHECK(octet_length(source_archive_sha256)=32),
    source_weekly_id BIGINT NOT NULL CHECK(source_weekly_id>0),
    source_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_row_sha256)=32),
    source_user_id BIGINT NOT NULL CHECK(source_user_id>0),
    source_conversation_id BIGINT NOT NULL CHECK(source_conversation_id>0),
    source_code_id BIGINT,
    original_status TEXT NOT NULL CHECK(original_status IN ('declined','offered','claimed')),
    original_claimed_at BIGINT,
    UNIQUE(source_namespace_sha256,source_weekly_id),
    CHECK((original_status='claimed' AND source_code_id IS NOT NULL AND source_code_id>0 AND original_claimed_at IS NOT NULL)
       OR (original_status<>'claimed' AND source_code_id IS NULL AND original_claimed_at IS NULL))
);
CREATE VIEW core_payments.weekly_discount_owners WITH(security_invoker=true) AS
    SELECT code_id,user_id AS owner_user_id FROM core_weekly.weekly_discounts
    WHERE code_id IS NOT NULL;

CREATE FUNCTION core_weekly.lock_weekly_discount_policy()
RETURNS TABLE(enabled BOOLEAN,maximum_percent SMALLINT)
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT p.enabled,p.maximum_percent FROM core_weekly.weekly_discount_policy p
    WHERE p.singleton FOR SHARE OF p
$$;

CREATE FUNCTION core_weekly.weekly_discount_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'weekly decision is durable'; END IF;
    IF ROW(NEW.id,NEW.user_id,NEW.account_id,NEW.week_start,NEW.origin,NEW.discount_percent,
       NEW.reason,NEW.conversation_id,NEW.operation_id,NEW.credential_id,NEW.auth_version,
       NEW.web_session_sid,NEW.created_at)
       IS DISTINCT FROM
       ROW(OLD.id,OLD.user_id,OLD.account_id,OLD.week_start,OLD.origin,OLD.discount_percent,
       OLD.reason,OLD.conversation_id,OLD.operation_id,OLD.credential_id,OLD.auth_version,
       OLD.web_session_sid,OLD.created_at)
       OR (ROW(NEW.status,NEW.code_id,NEW.claimed_at) IS DISTINCT FROM
           ROW(OLD.status,OLD.code_id,OLD.claimed_at)
           AND NOT(OLD.status='offered' AND NEW.status='claimed')) THEN
        RAISE EXCEPTION 'weekly decision identity or terminal result changed';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER weekly_discount_immutable BEFORE UPDATE OR DELETE
ON core_weekly.weekly_discounts FOR EACH ROW
EXECUTE FUNCTION core_weekly.weekly_discount_immutable();

CREATE FUNCTION core_weekly.weekly_discount_binding() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE d core_weekly.weekly_discounts%ROWTYPE;
BEGIN
    SELECT * INTO STRICT d FROM core_weekly.weekly_discounts WHERE id=NEW.id;
    IF d.origin='native' THEN
        IF NOT EXISTS(
            SELECT FROM core_assistant.agent_tool_operations o
            JOIN core_assistant.agent_turns t ON t.id=o.turn_id
            JOIN core_assistant.agent_rounds r ON r.id=o.round_id AND r.turn_id=t.id
            JOIN core_charging.requests q ON q.id=r.model_request_id
            WHERE o.id=d.operation_id AND o.state='completed' AND o.domain='registered_core'
              AND o.tool_name='prepare_weekly_discount' AND o.tool_version='core-weekly-discount-v1'
              AND o.domain_receipt_id='core-weekly-'||d.id
              AND o.result_sha256=sha256(o.result_json)
              AND o.arguments_sha256=sha256(o.arguments_json)
              AND (convert_from(o.arguments_json,'UTF8')::jsonb->>'discount_percent')::smallint=d.discount_percent
              AND btrim(convert_from(o.arguments_json,'UTF8')::jsonb->>'reason',U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000')=d.reason
              AND convert_from(o.result_json,'UTF8')::jsonb = jsonb_build_object(
                  'ok',true,'created',true,'decision',jsonb_build_object(
                      'id',d.id,'week_start',d.week_start,'expires_at',d.week_start+604800,
                      'discount_percent',d.discount_percent,'created_at',d.created_at,'claimed_at',NULL,
                      'browser_path','/api/user/auth/assistant/weekly-discounts/'||d.id,
                      'status',CASE WHEN d.discount_percent=0 THEN 'declined' ELSE 'offered' END))
              AND t.conversation_id=d.conversation_id AND t.owner_user_id=d.user_id
              AND t.owner_account_id=d.account_id AND t.session_id=d.credential_id
              AND t.auth_version=d.auth_version AND t.source_web_session_sid=d.web_session_sid
              AND r.state='settled' AND q.state='settled' AND q.outcome='completed'
              AND q.actor_user_id=d.user_id AND q.owner_account_id=d.account_id
        ) THEN RAISE EXCEPTION 'weekly decision requires the completed original Core tool'; END IF;
    ELSE
        IF NOT EXISTS(SELECT FROM core_weekly.legacy_weekly_discount_bases b
            WHERE b.decision_id=d.id
              AND (b.original_status=d.status OR (b.original_status='offered' AND d.status='claimed'))
              AND (b.original_status<>'claimed' OR (b.original_claimed_at=d.claimed_at AND EXISTS(
                  SELECT FROM core_payments.legacy_discount_bases c
                  WHERE c.code_id=d.code_id AND c.source_namespace_sha256=b.source_namespace_sha256
                    AND c.source_discount_id=b.source_code_id AND c.original_owner_user_id=d.user_id))))
        THEN RAISE EXCEPTION 'legacy weekly source or original coupon binding missing'; END IF;
    END IF;
    IF d.status='claimed' AND NOT EXISTS(
        SELECT FROM core_payments.discount_codes c WHERE c.id=d.code_id
          AND c.discount_percent=d.discount_percent AND c.max_uses=1 AND c.min_credit_units=0
          AND c.ends_at=d.week_start+604800
    ) THEN RAISE EXCEPTION 'weekly coupon terms differ from original decision'; END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER weekly_discount_binding AFTER INSERT OR UPDATE
ON core_weekly.weekly_discounts DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION core_weekly.weekly_discount_binding();

CREATE FUNCTION core_weekly.weekly_discount_tool_binding() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE result JSONB;
BEGIN
    IF NEW.tool_name='prepare_weekly_discount' AND NEW.state='completed' THEN
        result:=convert_from(NEW.result_json,'UTF8')::jsonb;
        IF result->>'created'='true' AND NOT EXISTS(
            SELECT FROM core_weekly.weekly_discounts d
            WHERE d.operation_id=NEW.id AND d.origin='native'
              AND NEW.domain_receipt_id='core-weekly-'||d.id
              AND result#>>'{decision,id}'=d.id
        ) THEN RAISE EXCEPTION 'weekly tool cannot report an uncommitted decision'; END IF;
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER weekly_discount_tool_binding AFTER INSERT OR UPDATE
ON core_assistant.agent_tool_operations DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION core_weekly.weekly_discount_tool_binding();

-- Runtime gets this bounded issuance operation, not general coupon administration.
-- Read a private code only through an already owned, claimed decision.
CREATE FUNCTION core_weekly.claimed_weekly_discount_code(decision TEXT,actor_user BIGINT,actor_account BIGINT)
RETURNS TEXT LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT c.code FROM core_weekly.weekly_discounts d
    JOIN core_payments.discount_codes c ON c.id=d.code_id
    WHERE d.id=decision AND d.user_id=actor_user AND d.account_id=actor_account AND d.status='claimed'
$$;

CREATE FUNCTION core_weekly.claim_weekly_discount_code(decision TEXT,new_code TEXT)
RETURNS TABLE(code_id BIGINT,code TEXT)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE d core_weekly.weekly_discounts%ROWTYPE; p RECORD; stamp BIGINT; issued BIGINT;
BEGIN
    SELECT * INTO STRICT d FROM core_weekly.weekly_discounts WHERE id=decision FOR UPDATE;
    IF d.status='claimed' THEN
        RETURN QUERY SELECT c.id,c.code FROM core_payments.discount_codes c WHERE c.id=d.code_id;
        RETURN;
    END IF;
    SELECT * INTO STRICT p FROM core_weekly.lock_weekly_discount_policy();
    stamp:=floor(extract(epoch FROM clock_timestamp()))::bigint;
    IF NOT p.enabled OR d.status<>'offered' OR d.discount_percent>p.maximum_percent
       OR stamp<d.week_start OR stamp>=d.week_start+604800
       OR new_code !~ '^AIWEEK-[0-9A-F]{32}$'
       OR NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
          WHERE u.id=d.user_id AND u.platform_role='user' AND u.active
            AND a.id=d.account_id AND a.kind='personal' AND a.active) THEN
        RAISE EXCEPTION 'weekly claim is not currently eligible';
    END IF;
    INSERT INTO core_payments.discount_codes(code,name,discount_percent,min_credit_units,max_uses,
        enabled,starts_at,ends_at,created_by,created_at)
    VALUES(new_code,'Assistant weekly discount',d.discount_percent,0,1,TRUE,
        stamp,d.week_start+604800,d.user_id,to_timestamp(stamp)) RETURNING id INTO issued;
    UPDATE core_weekly.weekly_discounts SET status='claimed',code_id=issued,claimed_at=stamp WHERE id=d.id;
    RETURN QUERY SELECT c.id,c.code FROM core_payments.discount_codes c WHERE c.id=issued;
END $$;

CREATE FUNCTION core_payments.weekly_coupon_terms_immutable() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    IF EXISTS(SELECT FROM core_weekly.weekly_discounts d WHERE d.code_id=OLD.id) THEN
        IF TG_OP='DELETE' OR ROW(NEW.id,NEW.code,NEW.discount_percent,NEW.min_credit_units,
             NEW.max_uses,NEW.starts_at,NEW.ends_at,NEW.created_by,NEW.created_at)
           IS DISTINCT FROM ROW(OLD.id,OLD.code,OLD.discount_percent,OLD.min_credit_units,
             OLD.max_uses,OLD.starts_at,OLD.ends_at,OLD.created_by,OLD.created_at)
        THEN RAISE EXCEPTION 'weekly coupon economic terms are immutable'; END IF;
    END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER weekly_coupon_terms_immutable BEFORE UPDATE OR DELETE
ON core_payments.discount_codes FOR EACH ROW
EXECUTE FUNCTION core_payments.weekly_coupon_terms_immutable();

REVOKE ALL ON core_weekly.weekly_discount_policy,core_weekly.legacy_weekly_discount_bases FROM PUBLIC;
REVOKE ALL ON FUNCTION core_weekly.claimed_weekly_discount_code(TEXT,BIGINT,BIGINT),
    core_weekly.lock_weekly_discount_policy(),
    core_weekly.claim_weekly_discount_code(TEXT,TEXT),
    core_weekly.weekly_discount_immutable(),core_weekly.weekly_discount_binding(),
    core_weekly.weekly_discount_tool_binding(),core_payments.weekly_coupon_terms_immutable() FROM PUBLIC;

CREATE FUNCTION core_meta.guard_legacy_weekly_discount_import() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE m JSONB:=NEW.material_utf8::jsonb; r JSONB:=NEW.review_utf8::jsonb;
    ir core_migration.import_runs%ROWTYPE; owner_name NAME;
BEGIN
    SELECT pg_get_userbyid(datdba) INTO owner_name FROM pg_database WHERE datname=current_database();
    IF current_user<>owner_name OR NOT EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin)
    THEN RAISE EXCEPTION 'weekly offline owner role required'; END IF;
    IF m->>'format' IS DISTINCT FROM 'lmm.current-weekly-discount-import.v1'
       OR m->>'scope' IS DISTINCT FROM 'current-legacy-weekly-discounts-v1'
       OR m->>'migration_id' IS DISTINCT FROM NEW.import_id
       OR jsonb_typeof(m->'mapping') IS DISTINCT FROM 'array'
       OR jsonb_typeof(m->'source'->'rows') IS DISTINCT FROM 'array'
       OR m->'target'->>'component_count' IS DISTINCT FROM '88'
    THEN RAISE EXCEPTION 'weekly import material invalid'; END IF;
    IF m->>'source_kind'='SYNTHETIC_WEEKLY_FIXTURE' THEN
        IF current_database() !~ '^lmm_runtime_keys_test_[a-z0-9_]{1,30}$'
           OR coalesce(m->>'fixture_nonce' !~ '^[a-f0-9]{64}$',TRUE)
           OR shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database')
              IS DISTINCT FROM 'lmm-runtime-keys-synthetic:'||(m->>'fixture_nonce')
           OR r IS DISTINCT FROM jsonb_build_object('scope','synthetic-weekly-test-only',
                'material_sha256',NEW.material_sha256,'production_authorization',false)
        THEN RAISE EXCEPTION 'weekly fixture target required'; END IF;
    ELSE
        IF m->>'source_kind' IS DISTINCT FROM 'VERIFIED_FULL_CANONICAL_SOURCE'
           OR r->>'scope' IS DISTINCT FROM 'reviewed-current-legacy-weekly-discounts-v1'
           OR r->>'material_sha256' IS DISTINCT FROM NEW.material_sha256
           OR r->'migration_actor_user_id' IS DISTINCT FROM m->'migration_actor_user_id'
           OR r->'source_fence_sha256' IS DISTINCT FROM m->'source_fence_sha256'
           OR r->'target' IS DISTINCT FROM m->'target'
           OR coalesce(m->>'source_fence_sha256' !~ '^[a-f0-9]{64}$',TRUE)
           OR m->'target'->>'database' IS DISTINCT FROM current_database()
           OR m->'target'->>'database_oid' IS DISTINCT FROM
              (SELECT oid::text FROM pg_database WHERE datname=current_database())
           OR m->'target'->>'system_identifier' IS DISTINCT FROM
              (SELECT system_identifier::text FROM pg_control_system())
           OR m->'target'->>'server_instance_nonce' IS DISTINCT FROM current_setting('lmm.migration_instance_nonce',true)
        THEN RAISE EXCEPTION 'weekly review source fence or target changed'; END IF;
    END IF;
    IF NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
        WHERE u.id=(m->>'migration_actor_user_id')::bigint AND u.active AND a.active AND u.platform_role='superadmin')
    THEN RAISE EXCEPTION 'weekly enabled migration actor required'; END IF;
    SELECT * INTO ir FROM core_migration.import_runs WHERE migration_id=m->'identity'->>'migration_id';
    IF NOT FOUND OR ir.manifest->>'scope' IS DISTINCT FROM 'current-identity-append-completion-v1'
       OR ir.source_archive_sha256 IS DISTINCT FROM m->'source'->>'source_archive_sha256'
       OR ir.mapping_sha256 IS DISTINCT FROM m->'identity'->>'mapping_sha256'
       OR encode(sha256(convert_to(ir.manifest::text,'UTF8')),'hex') IS DISTINCT FROM m->'identity'->>'manifest_sha256'
       OR ir.manifest->>'source_namespace_sha256' IS DISTINCT FROM m->>'source_namespace_sha256'
       OR ir.manifest->'material'->>'source_canonical_export_sha256' IS DISTINCT FROM m->'source'->>'source_export_sha256'
       OR ir.manifest->'material'->>'source_canonical_export_proof_sha256' IS DISTINCT FROM m->'source'->>'source_export_proof_sha256'
       OR ir.manifest->'actual_identity_authority' IS DISTINCT FROM m->'identity'->'actual_authority'
       OR core_meta.legacy_affiliate_alias_authority(m->'mapping') IS DISTINCT FROM m->'identity'->'actual_authority'
    THEN RAISE EXCEPTION 'weekly actual committed c343 identity receipt required'; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER legacy_weekly_discount_import_guard BEFORE INSERT
ON core_meta.legacy_weekly_discount_imports FOR EACH ROW
EXECUTE FUNCTION core_meta.guard_legacy_weekly_discount_import();

CREATE FUNCTION core_meta.legacy_weekly_discount_evidence_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'legacy weekly evidence is immutable'; END $$;
CREATE TRIGGER legacy_weekly_discount_import_immutable BEFORE UPDATE OR DELETE
ON core_meta.legacy_weekly_discount_imports FOR EACH ROW
EXECUTE FUNCTION core_meta.legacy_weekly_discount_evidence_immutable();
CREATE TRIGGER legacy_weekly_discount_basis_immutable BEFORE UPDATE OR DELETE
ON core_weekly.legacy_weekly_discount_bases FOR EACH ROW
EXECUTE FUNCTION core_meta.legacy_weekly_discount_evidence_immutable();

CREATE FUNCTION core_meta.complete_legacy_weekly_discount_import() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE m JSONB:=NEW.material; expected BIGINT;
BEGIN
    expected:=jsonb_array_length(m->'source'->'rows');
    IF (SELECT count(*) FROM core_weekly.legacy_weekly_discount_bases b WHERE b.import_id=NEW.import_id)<>expected
       OR (m->'source'->>'decision_count')::bigint<>expected
       OR EXISTS(
        SELECT FROM jsonb_array_elements(m->'source'->'rows') s
        LEFT JOIN jsonb_array_elements(m->'mapping') mp ON mp->>'source_user_id'=s->>'source_user_id'
        LEFT JOIN core_weekly.legacy_weekly_discount_bases b
          ON b.import_id=NEW.import_id AND b.source_weekly_id=(s->>'source_weekly_id')::bigint
        LEFT JOIN core_weekly.weekly_discounts d ON d.id=b.decision_id
        WHERE b.decision_id IS NULL OR mp IS NULL OR d.origin<>'legacy'
          OR d.user_id IS DISTINCT FROM (mp->>'target_user_id')::bigint
          OR d.account_id IS DISTINCT FROM (mp->>'target_personal_account_id')::bigint
          OR d.week_start IS DISTINCT FROM (s->>'week_start')::bigint
          OR d.status IS DISTINCT FROM s->>'status'
          OR d.discount_percent IS DISTINCT FROM (s->>'discount_percent')::smallint
          OR d.reason IS DISTINCT FROM s->>'reason'
          OR d.created_at IS DISTINCT FROM (s->>'created_at')::bigint
          OR d.claimed_at IS DISTINCT FROM (s->>'claimed_at')::bigint
          OR b.source_user_id IS DISTINCT FROM (s->>'source_user_id')::bigint
          OR b.original_status IS DISTINCT FROM s->>'status'
          OR b.original_claimed_at IS DISTINCT FROM (s->>'claimed_at')::bigint
          OR (s->>'status'='claimed' AND NOT EXISTS(
              SELECT FROM core_payments.legacy_discount_bases c
              WHERE c.code_id=d.code_id AND c.source_namespace_sha256=b.source_namespace_sha256
                AND c.source_archive_sha256=b.source_archive_sha256
                AND c.source_discount_id=(s->>'source_code_id')::bigint
                AND c.source_row_sha256=decode(s->>'source_coupon_row_sha256','hex')
                AND c.original_owner_user_id=d.user_id))
          OR b.source_conversation_id IS DISTINCT FROM (s->>'source_conversation_id')::bigint
          OR b.source_code_id IS DISTINCT FROM (s->>'source_code_id')::bigint
          OR b.source_row_sha256 IS DISTINCT FROM decode(s->>'source_row_sha256','hex')
          OR b.source_namespace_sha256 IS DISTINCT FROM decode(m->>'source_namespace_sha256','hex')
          OR b.source_archive_sha256 IS DISTINCT FROM decode(m->'source'->>'source_archive_sha256','hex')
       ) THEN RAISE EXCEPTION 'weekly source rows or owner mapping incomplete'; END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER legacy_weekly_discount_import_complete AFTER INSERT
ON core_meta.legacy_weekly_discount_imports DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION core_meta.complete_legacy_weekly_discount_import();

REVOKE ALL ON core_meta.legacy_weekly_discount_imports FROM PUBLIC;
REVOKE ALL ON FUNCTION core_meta.guard_legacy_weekly_discount_import(),
    core_meta.legacy_weekly_discount_evidence_immutable(),core_meta.complete_legacy_weekly_discount_import() FROM PUBLIC;

CREATE TABLE core_meta.assistant_weekly_discount_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER assistant_weekly_discount_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.assistant_weekly_discount_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER assistant_weekly_discount_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.assistant_weekly_discount_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.assistant_weekly_discount_upgrades FROM PUBLIC;
