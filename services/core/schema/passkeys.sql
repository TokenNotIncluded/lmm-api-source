-- Independent fresh-install component. Existing installations require the
-- schema owner's explicit versioned offline upgrade; never apply at startup.
CREATE SCHEMA core_passkeys;
CREATE TABLE core_passkeys.user_handles (
    user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
    user_handle UUID NOT NULL UNIQUE DEFAULT gen_random_uuid()
);
CREATE TABLE core_passkeys.credentials (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    credential_id BYTEA NOT NULL UNIQUE CHECK(octet_length(credential_id) BETWEEN 1 AND 4096),
    passkey JSONB NOT NULL CHECK(jsonb_typeof(passkey)='object' AND octet_length(passkey::text)<=65536),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64 AND octet_length(name)<=256),
    revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX passkeys_user ON core_passkeys.credentials(user_id,id);
CREATE TABLE core_passkeys.flows (
    digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
    binding_digest BYTEA NOT NULL CHECK(octet_length(binding_digest)=32),
    purpose TEXT NOT NULL CHECK(purpose IN ('register','login')),
    user_id BIGINT REFERENCES core_identity.users(id),
    user_version BIGINT CHECK(user_version>0),
    sid TEXT REFERENCES core_identity.web_sessions(sid),
    state JSONB NOT NULL CHECK(jsonb_typeof(state)='object' AND octet_length(state::text)<=131072),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()+interval '5 minutes',
    CHECK((purpose='register' AND user_id IS NOT NULL AND user_version IS NOT NULL AND sid IS NOT NULL)
        OR (purpose='login' AND sid IS NULL AND ((user_id IS NULL AND user_version IS NULL) OR (user_id IS NOT NULL AND user_version IS NOT NULL))))
);
CREATE INDEX passkeys_flows_expiry ON core_passkeys.flows(expires_at);

CREATE FUNCTION core_passkeys.guard_handle() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'immutable passkey user handle' USING ERRCODE='23514'; END $$;
CREATE TRIGGER passkey_handle_immutable BEFORE UPDATE OR DELETE ON core_passkeys.user_handles
    FOR EACH ROW EXECUTE FUNCTION core_passkeys.guard_handle();
CREATE TRIGGER passkey_handle_no_truncate BEFORE TRUNCATE ON core_passkeys.user_handles
    FOR EACH STATEMENT EXECUTE FUNCTION core_passkeys.guard_handle();
CREATE FUNCTION core_passkeys.guard_credential() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'UPDATE' THEN RAISE EXCEPTION 'retain passkey identity history' USING ERRCODE='23514'; END IF;
    IF NEW.id<>OLD.id OR NEW.user_id<>OLD.user_id OR NEW.credential_id<>OLD.credential_id OR NEW.created_at<>OLD.created_at
       OR (OLD.revoked_at IS NOT NULL AND NEW IS DISTINCT FROM OLD)
       OR NEW.revision<>OLD.revision+1 THEN
        RAISE EXCEPTION 'immutable passkey identity or invalid revision' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER passkey_credential_guard BEFORE UPDATE OR DELETE ON core_passkeys.credentials
    FOR EACH ROW EXECUTE FUNCTION core_passkeys.guard_credential();
CREATE TRIGGER passkey_credential_no_truncate BEFORE TRUNCATE ON core_passkeys.credentials
    FOR EACH STATEMENT EXECUTE FUNCTION core_passkeys.guard_handle();
REVOKE ALL ON SCHEMA core_passkeys FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_passkeys FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_passkeys FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_passkeys FROM PUBLIC;
-- Runtime grant inventory: schema USAGE; user_handles SELECT,INSERT(user_id);
-- credentials SELECT,INSERT(user_id,credential_id,passkey,name),
-- UPDATE(passkey,revision,last_used_at,revoked_at); flows SELECT,DELETE,
-- INSERT(digest,binding_digest,purpose,user_id,user_version,sid,state);
-- credentials_id_seq USAGE. No runtime ownership/DDL/TRUNCATE/DELETE on
-- credential history, nor UPDATE/DELETE on stable user handles.
