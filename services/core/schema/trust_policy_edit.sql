-- First part of the single trust-policy-edit component. These primitives are
-- owner-only; runtime access goes through the proof and editor wrappers.
CREATE FUNCTION core_trust_pricing.valid_discount_for_edit(value jsonb)
RETURNS boolean LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog AS $$
DECLARE
    n numeric;
    d numeric;
BEGIN
    IF jsonb_typeof(value) IS DISTINCT FROM 'object'
       OR NOT (value ?& ARRAY['numerator','denominator'])
       OR value - ARRAY['numerator','denominator'] <> '{}'::jsonb
       OR jsonb_typeof(value->'numerator') IS DISTINCT FROM 'number'
       OR jsonb_typeof(value->'denominator') IS DISTINCT FROM 'number'
       OR (value->>'numerator') !~ '^[0-9]{1,19}$'
       OR (value->>'denominator') !~ '^[0-9]{1,19}$' THEN
        RETURN false;
    END IF;
    n := (value->>'numerator')::numeric;
    d := (value->>'denominator')::numeric;
    RETURN n > 0 AND n <= d AND d <= 1000000000000000000;
EXCEPTION WHEN data_exception THEN
    RETURN false;
END $$;

CREATE FUNCTION core_trust_pricing.valid_policy_for_edit(value jsonb)
RETURNS boolean LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog AS $$
DECLARE
    tier jsonb;
    i integer;
    amount numeric;
    previous numeric := -1;
BEGIN
    IF jsonb_typeof(value) IS DISTINCT FROM 'object'
       OR NOT (value ?& ARRAY['version','paid_activation_enabled','decay_period_days','tiers','admin_discount','superadmin_discount'])
       OR value - ARRAY['version','paid_activation_enabled','decay_period_days','tiers','admin_discount','superadmin_discount'] <> '{}'::jsonb
       OR jsonb_typeof(value->'version') IS DISTINCT FROM 'number'
       OR value->>'version' IS DISTINCT FROM '1'
       OR jsonb_typeof(value->'paid_activation_enabled') IS DISTINCT FROM 'boolean'
       OR jsonb_typeof(value->'decay_period_days') IS DISTINCT FROM 'number'
       OR (value->>'decay_period_days') !~ '^[0-9]{1,4}$'
       OR (value->>'decay_period_days')::numeric > 3650
       OR jsonb_typeof(value->'tiers') IS DISTINCT FROM 'array' THEN
        RETURN false;
    END IF;
    IF jsonb_array_length(value->'tiers') <> 5
       OR NOT core_trust_pricing.valid_discount_for_edit(value->'admin_discount')
       OR NOT core_trust_pricing.valid_discount_for_edit(value->'superadmin_discount') THEN
        RETURN false;
    END IF;
    FOR i IN 0..4 LOOP
        tier := (value->'tiers')->i;
        IF jsonb_typeof(tier) IS DISTINCT FROM 'object'
           OR NOT (tier ?& ARRAY['level','min_paid_credits','discount'])
           OR tier - ARRAY['level','min_paid_credits','discount'] <> '{}'::jsonb
           OR jsonb_typeof(tier->'level') IS DISTINCT FROM 'number'
           OR tier->>'level' IS DISTINCT FROM i::text
           OR jsonb_typeof(tier->'min_paid_credits') IS DISTINCT FROM 'number'
           OR (tier->>'min_paid_credits') !~ '^[0-9]{1,16}$'
           OR NOT core_trust_pricing.valid_discount_for_edit(tier->'discount') THEN
            RETURN false;
        END IF;
        amount := (tier->>'min_paid_credits')::numeric;
        IF amount > 9007199254740991 OR (i = 0 AND amount <> 0)
           OR (i > 0 AND amount <= previous) THEN
            RETURN false;
        END IF;
        previous := amount;
    END LOOP;
    RETURN true;
EXCEPTION WHEN data_exception THEN
    RETURN false;
END $$;
-- Server-issued, one-use previews are staged without any runtime grants. The
-- token itself is never stored, only its 32-byte digest. Authority and expiry
-- must still be rechecked inside the future commit transaction.
CREATE TABLE core_trust_pricing.policy_edit_previews (
    token_digest bytea PRIMARY KEY CHECK (octet_length(token_digest) = 32),
    actor_user_id bigint NOT NULL REFERENCES core_identity.users(id),
    session_credential_id bigint NOT NULL REFERENCES core_identity.credentials(id),
    base_revision bigint NOT NULL CHECK (base_revision BETWEEN 1 AND 9223372036854775806),
    before_spec jsonb NOT NULL CHECK (core_trust_pricing.valid_policy_for_edit(before_spec)),
    after_spec jsonb NOT NULL CHECK (core_trust_pricing.valid_policy_for_edit(after_spec)),
    before_console jsonb CHECK (before_console IS NULL OR jsonb_typeof(before_console) = 'object'),
    after_console jsonb NOT NULL CHECK (jsonb_typeof(after_console) = 'object'),
    transition_digest bytea NOT NULL CHECK (octet_length(transition_digest) = 32),
    created_at timestamptz NOT NULL DEFAULT statement_timestamp(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    resulting_revision bigint CHECK (resulting_revision = base_revision + 1),
    CHECK (expires_at > created_at AND expires_at <= created_at + interval '15 minutes'),
    CHECK ((consumed_at IS NULL) = (resulting_revision IS NULL)),
    CHECK (consumed_at IS NULL OR (consumed_at >= created_at AND consumed_at < expires_at))
);

CREATE INDEX policy_edit_active_session ON core_trust_pricing.policy_edit_previews(session_credential_id,expires_at)
    WHERE consumed_at IS NULL;

-- The eventual update and this audit insert must be one transaction under the
-- same policy lock. Initial imported console descriptions may be NULL/unknown.
CREATE TABLE core_trust_pricing.policy_edit_audit (
    revision bigint PRIMARY KEY CHECK (revision > 1),
    actor_user_id bigint NOT NULL REFERENCES core_identity.users(id),
    session_credential_id bigint NOT NULL REFERENCES core_identity.credentials(id),
    preview_digest bytea NOT NULL UNIQUE REFERENCES core_trust_pricing.policy_edit_previews(token_digest),
    transition_digest bytea NOT NULL CHECK (octet_length(transition_digest) = 32),
    before_source_digest bytea NOT NULL CHECK (octet_length(before_source_digest) = 32),
    after_source_digest bytea NOT NULL CHECK (octet_length(after_source_digest) = 32),
    before_spec jsonb NOT NULL CHECK (core_trust_pricing.valid_policy_for_edit(before_spec)),
    after_spec jsonb NOT NULL CHECK (core_trust_pricing.valid_policy_for_edit(after_spec)),
    before_console jsonb CHECK (before_console IS NULL OR jsonb_typeof(before_console) = 'object'),
    after_console jsonb NOT NULL CHECK (jsonb_typeof(after_console) = 'object'),
    changed_at timestamptz NOT NULL DEFAULT statement_timestamp()
);
CREATE FUNCTION core_trust_pricing.prevent_policy_edit_audit_mutation()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'policy edit audit is append-only' USING ERRCODE = '55000';
END $$;
CREATE TRIGGER policy_edit_audit_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON core_trust_pricing.policy_edit_audit
    FOR EACH STATEMENT EXECUTE FUNCTION core_trust_pricing.prevent_policy_edit_audit_mutation();
-- Staged primitives only: no runtime EXECUTE grant or HTTP wiring. The caller
-- must have already authenticated the session and locked current MFA/authority
-- through Identity in the same transaction; IDs alone are not authentication.
-- The application must construct/validate both console descriptions and the
-- digest and provide a cryptographically random, secret preview token digest.
CREATE FUNCTION core_trust_pricing.read_policy_edit_description(p_revision bigint)
RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT a.after_console FROM core_trust_pricing.policy_edit_audit a
    JOIN core_trust_pricing.policies p ON p.singleton AND p.revision=a.revision
    WHERE a.revision=p_revision
$$;
REVOKE ALL ON FUNCTION core_trust_pricing.read_policy_edit_description(bigint) FROM PUBLIC;

CREATE FUNCTION core_trust_pricing.create_policy_edit_preview(
    p_token_digest bytea, p_actor bigint, p_credential bigint, p_base bigint,
    p_before jsonb, p_after jsonb, p_before_console jsonb, p_after_console jsonb,
    p_transition_digest bytea
) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    current_revision bigint;
    current_spec jsonb;
    now_at timestamptz;
BEGIN
    IF octet_length(p_token_digest) IS DISTINCT FROM 32
       OR octet_length(p_transition_digest) IS DISTINCT FROM 32
       OR NOT core_trust_pricing.valid_policy_for_edit(p_before)
       OR NOT core_trust_pricing.valid_policy_for_edit(p_after)
       OR jsonb_typeof(p_after_console) IS DISTINCT FROM 'object'
       OR (p_before_console IS NOT NULL AND jsonb_typeof(p_before_console) IS DISTINCT FROM 'object')
       OR octet_length(p_after_console::text) > 16384
       OR (p_before_console IS NOT NULL AND octet_length(p_before_console::text) > 16384) THEN
        RAISE EXCEPTION 'invalid policy preview' USING ERRCODE = '22023';
    END IF;
    PERFORM 1 FROM core_identity.credentials c
    JOIN core_identity.users u ON u.id=c.user_id
    JOIN core_identity.accounts a ON a.id=u.personal_account_id
    WHERE c.id=p_credential AND c.user_id=p_actor AND c.kind='session'
      AND c.owner_account_id=u.personal_account_id AND c.revoked_at IS NULL
      AND c.expires_at > clock_timestamp() AND c.user_version=u.auth_version
      AND u.active AND u.platform_role='superadmin' AND a.active
    FOR SHARE OF c,u,a;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid policy edit authority' USING ERRCODE = '42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(741803219);
    SELECT revision,spec INTO current_revision,current_spec
        FROM core_trust_pricing.policies WHERE singleton;
    IF NOT FOUND OR current_revision IS DISTINCT FROM p_base
       OR current_spec IS DISTINCT FROM p_before THEN
        RAISE EXCEPTION 'stale policy preview' USING ERRCODE = '40001';
    END IF;
    now_at := clock_timestamp();
    IF NOT EXISTS (SELECT 1 FROM core_identity.credentials c
        WHERE c.id=p_credential AND c.expires_at > now_at AND c.revoked_at IS NULL) THEN
        RAISE EXCEPTION 'expired policy edit authority' USING ERRCODE = '42501';
    END IF;
    -- Bound concurrent outstanding previews for a session. Serialize the count
    -- across transactions; modulo collisions only reduce concurrency.
    PERFORM pg_catalog.pg_advisory_xact_lock(741803220, (p_credential % 2147483647)::integer);
    DELETE FROM core_trust_pricing.policy_edit_previews
        WHERE session_credential_id=p_credential AND consumed_at IS NULL AND expires_at <= now_at;
    IF (SELECT count(*) FROM core_trust_pricing.policy_edit_previews
        WHERE session_credential_id=p_credential AND consumed_at IS NULL AND expires_at > now_at) >= 8 THEN
        RAISE EXCEPTION 'too many pending policy previews' USING ERRCODE = '54000';
    END IF;
    INSERT INTO core_trust_pricing.policy_edit_previews(
        token_digest,actor_user_id,session_credential_id,base_revision,before_spec,
        after_spec,before_console,after_console,transition_digest,created_at,expires_at
    ) VALUES (p_token_digest,p_actor,p_credential,p_base,p_before,p_after,p_before_console,
        p_after_console,p_transition_digest,now_at,now_at+interval '5 minutes');
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.create_policy_edit_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea) FROM PUBLIC;

CREATE FUNCTION core_trust_pricing.apply_policy_edit(
    p_token_digest bytea, p_actor bigint, p_credential bigint, p_transition_digest bytea
) RETURNS bigint LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    candidate core_trust_pricing.policy_edit_previews%ROWTYPE;
    old_policy core_trust_pricing.policies%ROWTYPE;
    now_at timestamptz;
    new_revision bigint;
    new_source bytea;
BEGIN
    IF octet_length(p_token_digest) IS DISTINCT FROM 32
       OR octet_length(p_transition_digest) IS DISTINCT FROM 32 THEN
        RAISE EXCEPTION 'invalid policy edit' USING ERRCODE = '22023';
    END IF;
    -- Match Identity's lock class before taking the pricing lock. This is
    -- defense-in-depth, not a substitute for credential authentication or MFA.
    PERFORM 1 FROM core_identity.credentials c
    JOIN core_identity.users u ON u.id=c.user_id
    JOIN core_identity.accounts a ON a.id=u.personal_account_id
    WHERE c.id=p_credential AND c.user_id=p_actor AND c.kind='session'
      AND c.owner_account_id=u.personal_account_id AND c.revoked_at IS NULL
      AND c.expires_at > clock_timestamp() AND c.user_version=u.auth_version
      AND u.active AND u.platform_role='superadmin' AND a.active
    FOR SHARE OF c,u,a;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid policy edit authority' USING ERRCODE = '42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(741803219);
    SELECT * INTO candidate FROM core_trust_pricing.policy_edit_previews
        WHERE token_digest=p_token_digest FOR UPDATE;
    now_at := clock_timestamp();
    IF NOT FOUND OR candidate.actor_user_id <> p_actor
       OR candidate.session_credential_id <> p_credential
       OR candidate.transition_digest <> p_transition_digest
       OR candidate.consumed_at IS NOT NULL OR candidate.expires_at <= now_at THEN
        RAISE EXCEPTION 'invalid or expired policy preview' USING ERRCODE = '22023';
    END IF;
    SELECT * INTO old_policy FROM core_trust_pricing.policies WHERE singleton FOR UPDATE;
    IF NOT FOUND OR old_policy.revision <> candidate.base_revision
       OR old_policy.spec IS DISTINCT FROM candidate.before_spec THEN
        RAISE EXCEPTION 'stale policy preview' USING ERRCODE = '40001';
    END IF;
    -- A lock wait may have crossed expiry even though the initial check passed.
    now_at := clock_timestamp();
    IF candidate.expires_at <= now_at OR NOT EXISTS (
        SELECT 1 FROM core_identity.credentials c
        WHERE c.id=p_credential AND c.expires_at > now_at AND c.revoked_at IS NULL
    ) THEN
        RAISE EXCEPTION 'expired policy edit authority' USING ERRCODE = '42501';
    END IF;
    new_revision := old_policy.revision + 1;
    new_source := sha256(convert_to('core-trust-policy-edit-v1', 'UTF8') || candidate.transition_digest);
    UPDATE core_trust_pricing.policies SET revision=new_revision,
        spec=candidate.after_spec, source_digest=new_source WHERE singleton;
    INSERT INTO core_trust_pricing.policy_edit_audit(
        revision,actor_user_id,session_credential_id,preview_digest,transition_digest,
        before_source_digest,after_source_digest,before_spec,after_spec,before_console,after_console,changed_at
    ) VALUES (new_revision,p_actor,p_credential,p_token_digest,p_transition_digest,
        old_policy.source_digest,new_source,candidate.before_spec,candidate.after_spec,
        candidate.before_console,candidate.after_console,now_at);
    UPDATE core_trust_pricing.policy_edit_previews
        SET consumed_at=now_at,resulting_revision=new_revision WHERE token_digest=p_token_digest;
    RETURN new_revision;
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.apply_policy_edit(bytea,bigint,bigint,bytea) FROM PUBLIC;

-- The caller locks a current authenticated session for the same actor. A fresh
-- session may resolve an old token, but may never confirm that old preview.
-- The exclusive fence waits for a possible still-running preview or commit.
CREATE FUNCTION core_trust_pricing.resolve_policy_edit(p_token_digest bytea,p_actor bigint,p_credential bigint)
RETURNS TABLE(state text, revision bigint) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE candidate core_trust_pricing.policy_edit_previews%ROWTYPE;
BEGIN
    IF octet_length(p_token_digest) IS DISTINCT FROM 32 THEN
        RAISE EXCEPTION 'invalid policy edit' USING ERRCODE = '22023';
    END IF;
    PERFORM 1 FROM core_identity.credentials c
    JOIN core_identity.users u ON u.id=c.user_id
    JOIN core_identity.accounts a ON a.id=u.personal_account_id
    WHERE c.id=p_credential AND c.user_id=p_actor AND c.kind='session'
      AND c.owner_account_id=u.personal_account_id AND c.revoked_at IS NULL
      AND c.expires_at > clock_timestamp() AND c.user_version=u.auth_version
      AND u.active AND u.platform_role='superadmin' AND a.active
    FOR SHARE OF c,u,a;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid policy edit authority' USING ERRCODE = '42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(741803219);
    IF NOT EXISTS (SELECT 1 FROM core_identity.credentials c
        WHERE c.id=p_credential AND c.expires_at > clock_timestamp() AND c.revoked_at IS NULL) THEN
        RAISE EXCEPTION 'expired policy edit authority' USING ERRCODE = '42501';
    END IF;
    SELECT * INTO candidate FROM core_trust_pricing.policy_edit_previews
      WHERE token_digest=p_token_digest AND actor_user_id=p_actor;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'unknown'::text,NULL::bigint;
    ELSIF candidate.consumed_at IS NOT NULL THEN
        RETURN QUERY SELECT 'confirmed'::text,candidate.resulting_revision;
    ELSIF candidate.expires_at <= clock_timestamp() THEN
        RETURN QUERY SELECT 'expired'::text,NULL::bigint;
    ELSE
        RETURN QUERY SELECT 'pending'::text,NULL::bigint;
    END IF;
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.resolve_policy_edit(bytea,bigint,bigint) FROM PUBLIC;
REVOKE ALL ON core_trust_pricing.policy_edit_previews, core_trust_pricing.policy_edit_audit FROM PUBLIC;
REVOKE ALL ON FUNCTION core_trust_pricing.prevent_policy_edit_audit_mutation() FROM PUBLIC;
REVOKE ALL ON FUNCTION core_trust_pricing.valid_discount_for_edit(jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION core_trust_pricing.valid_policy_for_edit(jsonb) FROM PUBLIC;
