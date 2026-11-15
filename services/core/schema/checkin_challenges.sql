-- Core-owned interactive authorization; installed only by the schema owner.
CREATE SCHEMA core_checkin_challenges;
CREATE TABLE core_checkin_challenges.challenges (
    digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    sid TEXT NOT NULL REFERENCES core_identity.web_sessions(sid),
    user_version BIGINT NOT NULL CHECK(user_version>0),
    operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
    day DATE NOT NULL,
    passkey_required BOOLEAN NOT NULL,
    passkey_state JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()+interval '120 seconds',
    attempts SMALLINT NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 3),
    verified_at TIMESTAMPTZ,
    consumed_at TIMESTAMPTZ,
    CHECK(expires_at>created_at AND expires_at<=created_at+interval '121 seconds'),
    CHECK((passkey_required AND passkey_state IS NOT NULL AND jsonb_typeof(passkey_state)='object' AND octet_length(passkey_state::text)<=131072)
        OR (NOT passkey_required AND passkey_state IS NULL)),
    CHECK(consumed_at IS NULL OR verified_at IS NOT NULL)
);
CREATE INDEX checkin_challenge_expiry ON core_checkin_challenges.challenges(expires_at);
CREATE INDEX checkin_challenge_user_time ON core_checkin_challenges.challenges(user_id,created_at);
CREATE TABLE core_checkin_challenges.receipts (
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    day DATE NOT NULL,
    amount_units BIGINT NOT NULL CHECK(amount_units>=0),
    challenge_digest BYTEA NOT NULL UNIQUE CHECK(octet_length(challenge_digest)=32),
    operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
    journal_id BIGINT REFERENCES core_billing.ledger_journals(id),
    timestamp BIGINT NOT NULL CHECK(timestamp>0),
    PRIMARY KEY(account_id,day),
    UNIQUE(user_id,day),
    CHECK((amount_units=0)=(journal_id IS NULL))
);
CREATE FUNCTION core_checkin_challenges.guard_challenge() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        IF OLD.expires_at>clock_timestamp() THEN RAISE EXCEPTION 'active checkin challenge' USING ERRCODE='23514'; END IF;
        RETURN OLD;
    END IF;
    IF (to_jsonb(NEW)-'attempts'-'verified_at'-'consumed_at') IS DISTINCT FROM
       (to_jsonb(OLD)-'attempts'-'verified_at'-'consumed_at')
       OR NEW.attempts<OLD.attempts OR NEW.attempts>OLD.attempts+1
       OR (OLD.verified_at IS NOT NULL AND NEW.verified_at IS DISTINCT FROM OLD.verified_at)
       OR (OLD.consumed_at IS NOT NULL AND NEW IS DISTINCT FROM OLD) THEN
        RAISE EXCEPTION 'immutable checkin challenge binding' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER checkin_challenge_guard BEFORE UPDATE OR DELETE ON core_checkin_challenges.challenges
    FOR EACH ROW EXECUTE FUNCTION core_checkin_challenges.guard_challenge();
CREATE FUNCTION core_checkin_challenges.immutable_receipt() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'immutable checkin receipt' USING ERRCODE='23514'; END $$;
CREATE TRIGGER checkin_receipt_immutable BEFORE UPDATE OR DELETE ON core_checkin_challenges.receipts
    FOR EACH ROW EXECUTE FUNCTION core_checkin_challenges.immutable_receipt();
CREATE TRIGGER checkin_receipt_no_truncate BEFORE TRUNCATE ON core_checkin_challenges.receipts
    FOR EACH STATEMENT EXECUTE FUNCTION core_checkin_challenges.immutable_receipt();
REVOKE ALL ON SCHEMA core_checkin_challenges FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_checkin_challenges FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_checkin_challenges FROM PUBLIC;
-- Runtime: schema USAGE; challenges SELECT,INSERT,DELETE,
-- UPDATE(attempts,verified_at,consumed_at); receipts SELECT,INSERT.

-- Owner-only offline 74 -> 75 receipt, never runtime upgrade authority.
CREATE TABLE core_meta.checkin_challenges_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER checkin_challenges_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.checkin_challenges_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER checkin_challenges_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.checkin_challenges_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.checkin_challenges_upgrades FROM PUBLIC;
