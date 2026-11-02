-- Installed as part of trust-policy-edit; key provisioning is a separate owner operation.
-- Proof blocks direct invocation using only guessed credential IDs, provided
-- the signing key is secret. It does NOT protect against a runtime DB role
-- that can modify identity state and then induce the service to sign as root.
-- The editor component additionally requires independent WebAuthn verification
-- before signing confirmation; preview/resolve signatures alone cannot commit.
-- A separate 32-byte application signing key is held only by the service
-- process and this owner-only table. Signatures bind
-- an exact action/payload to one PostgreSQL backend and top-level transaction;
-- a logged signature from an earlier transaction cannot be replayed.
CREATE TABLE core_trust_pricing.policy_edit_signing_key (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    secret bytea NOT NULL CHECK (octet_length(secret) = 32)
);
REVOKE ALL ON core_trust_pricing.policy_edit_signing_key FROM PUBLIC;

-- Standard RFC 2104 HMAC-SHA256; PostgreSQL's built-in sha256(bytea) requires
-- no additional extension. The caller must supply a 32-byte key.
CREATE FUNCTION core_trust_pricing.policy_edit_hmac(key_bytes bytea, message bytea)
RETURNS bytea LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE
    inner_pad bytea := decode(repeat('36', 64), 'hex');
    outer_pad bytea := decode(repeat('5c', 64), 'hex');
    i integer;
BEGIN
    IF octet_length(key_bytes) <> 32 THEN
        RAISE EXCEPTION 'invalid signing key' USING ERRCODE = '22023';
    END IF;
    FOR i IN 0..31 LOOP
        inner_pad := set_byte(inner_pad, i, get_byte(inner_pad, i) # get_byte(key_bytes, i));
        outer_pad := set_byte(outer_pad, i, get_byte(outer_pad, i) # get_byte(key_bytes, i));
    END LOOP;
    RETURN sha256(outer_pad || sha256(inner_pad || message));
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.policy_edit_hmac(bytea,bytea) FROM PUBLIC;

-- This digest contains no signing key or credential. The action and complete
-- typed request payload must be recomputed inside each privileged function.
CREATE FUNCTION core_trust_pricing.policy_edit_binding(action text, payload jsonb)
RETURNS bytea LANGUAGE sql VOLATILE STRICT SET search_path=pg_catalog AS $$
    SELECT sha256(convert_to(jsonb_build_array(
        'lmm-trust-policy-edit-proof-v1', pg_backend_pid(), txid_current(), action, payload
    )::text, 'UTF8'))
$$;
REVOKE ALL ON FUNCTION core_trust_pricing.policy_edit_binding(text,jsonb) FROM PUBLIC;

-- Called only from SECURITY DEFINER operations; not granted to the runtime.
CREATE FUNCTION core_trust_pricing.policy_edit_verify(action text, payload jsonb, proof bytea)
RETURNS boolean LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE
    expected bytea;
    difference integer := 0;
    i integer;
BEGIN
    IF action IS NULL OR payload IS NULL OR octet_length(proof) IS DISTINCT FROM 32 THEN
        RETURN false;
    END IF;
    SELECT core_trust_pricing.policy_edit_hmac(secret,
        core_trust_pricing.policy_edit_binding(action, payload)) INTO expected
        FROM core_trust_pricing.policy_edit_signing_key WHERE singleton;
    IF expected IS NULL THEN
        RETURN false;
    END IF;
    -- Always compare all bytes; never expose the position of a mismatch.
    FOR i IN 0..31 LOOP
        difference := difference | (get_byte(expected, i) # get_byte(proof, i));
    END LOOP;
    RETURN difference = 0;
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.policy_edit_verify(text,jsonb,bytea) FROM PUBLIC;

-- The old implementation functions remain owner-only. Only these wrappers,
-- once explicitly granted, may be called by a restricted application role.
CREATE FUNCTION core_trust_pricing.authorized_policy_preview(
    p_token bytea, p_actor bigint, p_credential bigint, p_base bigint,
    p_before jsonb, p_after jsonb, p_before_console jsonb, p_after_console jsonb,
    p_transition bytea, p_proof bytea
) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    IF NOT core_trust_pricing.policy_edit_verify('preview',jsonb_build_array(
        encode(p_token,'hex'),p_actor,p_credential,p_base,p_before,p_after,
        p_before_console,p_after_console,encode(p_transition,'hex')),p_proof) THEN
        RAISE EXCEPTION 'invalid policy proof' USING ERRCODE = '42501';
    END IF;
    PERFORM core_trust_pricing.create_policy_edit_preview(p_token,p_actor,p_credential,p_base,
        p_before,p_after,p_before_console,p_after_console,p_transition);
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.authorized_policy_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea,bytea) FROM PUBLIC;

CREATE FUNCTION core_trust_pricing.authorized_policy_resolve(
    p_token bytea, p_actor bigint, p_credential bigint, p_proof bytea
) RETURNS TABLE(state text, revision bigint) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    IF NOT core_trust_pricing.policy_edit_verify('resolve',jsonb_build_array(
        encode(p_token,'hex'),p_actor,p_credential),p_proof) THEN
        RAISE EXCEPTION 'invalid policy proof' USING ERRCODE = '42501';
    END IF;
    RETURN QUERY SELECT * FROM core_trust_pricing.resolve_policy_edit(p_token,p_actor,p_credential);
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.authorized_policy_resolve(bytea,bigint,bigint,bytea) FROM PUBLIC;
