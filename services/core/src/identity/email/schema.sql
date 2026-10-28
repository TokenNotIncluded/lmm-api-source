-- Staged identity-v5 delta for isolated development only.
-- NOT included by init-db/startup. Do not apply to an installed Core database:
-- the reviewed identity revision, complete catalog receipt and grants are pending.
ALTER TABLE core_identity.user_profiles
    ADD COLUMN email_version BIGINT NOT NULL DEFAULT 0 CHECK (email_version >= 0);

-- Profile text is deliberately not copied into this authority.
CREATE TABLE core_identity.email_addresses (
    email_key TEXT PRIMARY KEY CHECK (email_key = lower(btrim(email_key)) AND length(email_key) BETWEEN 3 AND 254),
    user_id BIGINT UNIQUE REFERENCES core_identity.users(id),
    state TEXT NOT NULL CHECK (state IN ('verified','legacy_unverified','conflict')),
    email_version BIGINT NOT NULL CHECK (email_version >= 0),
    verified_at TIMESTAMPTZ,
    CHECK ((state = 'conflict') = (user_id IS NULL)),
    CHECK ((state = 'verified') = (verified_at IS NOT NULL)),
    CHECK (state <> 'verified' OR email_version > 0)
);

CREATE TABLE core_identity.email_challenges (
    id TEXT PRIMARY KEY CHECK (id ~ '^[a-f0-9]{32}$'),
    scope_key BYTEA NOT NULL CHECK (octet_length(scope_key) = 32),
    purpose TEXT NOT NULL CHECK (purpose IN ('register','bind','password_reset')),
    email_key TEXT NOT NULL CHECK (email_key = lower(btrim(email_key)) AND length(email_key) BETWEEN 3 AND 254),
    user_id BIGINT REFERENCES core_identity.users(id),
    user_version BIGINT CHECK (user_version > 0),
    email_version BIGINT CHECK (email_version >= 0),
    sid_digest BYTEA CHECK (octet_length(sid_digest) = 32),
    key_id TEXT NOT NULL CHECK (length(key_id) BETWEEN 1 AND 64),
    token_mac BYTEA NOT NULL CHECK (octet_length(token_mac) = 32),
    attempts SMALLINT NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','consumed','locked','expired','superseded')),
    issued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp() + interval '600 seconds',
    consumed_at TIMESTAMPTZ,
    result_user_id BIGINT REFERENCES core_identity.users(id),
    CHECK (expires_at > issued_at AND expires_at <= issued_at + interval '600 seconds'),
    CHECK ((purpose = 'register' AND user_id IS NULL AND user_version IS NULL AND email_version IS NULL)
        OR (purpose <> 'register' AND user_id IS NOT NULL AND user_version IS NOT NULL AND email_version IS NOT NULL)),
    CHECK ((purpose = 'bind') = (sid_digest IS NOT NULL)),
    CHECK ((state = 'consumed') = (consumed_at IS NOT NULL AND result_user_id IS NOT NULL)),
    CHECK (state = 'consumed' OR (consumed_at IS NULL AND result_user_id IS NULL)),
    CHECK (state <> 'consumed' OR consumed_at < expires_at),
    CHECK (state <> 'locked' OR attempts = 5)
);
CREATE UNIQUE INDEX email_one_active_scope ON core_identity.email_challenges(scope_key) WHERE state = 'active';

CREATE FUNCTION core_identity.protect_email_challenge() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF (NEW.id,NEW.scope_key,NEW.purpose,NEW.email_key,NEW.user_id,NEW.user_version,
        NEW.email_version,NEW.sid_digest,NEW.key_id,NEW.token_mac,NEW.issued_at,NEW.expires_at)
        IS DISTINCT FROM
       (OLD.id,OLD.scope_key,OLD.purpose,OLD.email_key,OLD.user_id,OLD.user_version,
        OLD.email_version,OLD.sid_digest,OLD.key_id,OLD.token_mac,OLD.issued_at,OLD.expires_at)
       OR NEW.attempts < OLD.attempts
       OR (OLD.state <> 'active' AND NEW IS DISTINCT FROM OLD) THEN
        RAISE EXCEPTION 'email challenge identity and terminal state are immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER email_challenge_guard BEFORE UPDATE ON core_identity.email_challenges
    FOR EACH ROW EXECUTE FUNCTION core_identity.protect_email_challenge();

CREATE TABLE core_identity.email_rate_buckets (
    bucket BYTEA PRIMARY KEY CHECK (octet_length(bucket) = 32)
);
CREATE TABLE core_identity.email_rate_events (
    bucket BYTEA NOT NULL REFERENCES core_identity.email_rate_buckets(bucket),
    kind TEXT NOT NULL CHECK (kind IN ('issue','verify')),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX email_rate_window ON core_identity.email_rate_events(bucket,kind,occurred_at);
REVOKE ALL ON core_identity.email_addresses,core_identity.email_challenges,
    core_identity.email_rate_buckets,core_identity.email_rate_events FROM PUBLIC;
