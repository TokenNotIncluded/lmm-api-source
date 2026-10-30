-- Independent native-account binding component. Never delete the historical
-- OAuth identity or free its provider subject for another account to claim.
CREATE SCHEMA core_account_links;
CREATE TABLE core_account_links.oauth_states (
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
    revoked_at TIMESTAMPTZ,
    PRIMARY KEY(issuer,subject),
    FOREIGN KEY(issuer,subject) REFERENCES core_identity.oauth_identities(issuer,subject)
);
CREATE FUNCTION core_account_links.protect_state() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP IN ('DELETE','TRUNCATE') THEN
        RAISE EXCEPTION 'retain OAuth identity history' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM core_identity.oauth_identities i WHERE i.issuer=NEW.issuer AND i.subject=NEW.subject AND i.user_id=NEW.user_id) THEN
        RAISE EXCEPTION 'OAuth owner mismatch' USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' AND ((NEW.issuer,NEW.subject,NEW.user_id) IS DISTINCT FROM (OLD.issuer,OLD.subject,OLD.user_id)
        OR NEW.revision<>OLD.revision+1) THEN
        RAISE EXCEPTION 'immutable OAuth identity or invalid revision' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER oauth_state_guard BEFORE INSERT OR UPDATE OR DELETE ON core_account_links.oauth_states
    FOR EACH ROW EXECUTE FUNCTION core_account_links.protect_state();
CREATE TRIGGER oauth_state_no_truncate BEFORE TRUNCATE ON core_account_links.oauth_states
    FOR EACH STATEMENT EXECUTE FUNCTION core_account_links.protect_state();
CREATE TABLE core_account_links.bind_flows (
    state_digest BYTEA PRIMARY KEY CHECK(octet_length(state_digest)=32),
    issuer TEXT NOT NULL,
    binding_digest BYTEA NOT NULL CHECK(octet_length(binding_digest)=32),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    user_version BIGINT NOT NULL CHECK(user_version>0),
    sid TEXT NOT NULL REFERENCES core_identity.web_sessions(sid),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()+interval '10 minutes'
);
CREATE INDEX account_bind_expiry ON core_account_links.bind_flows(expires_at);
REVOKE ALL ON SCHEMA core_account_links FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_account_links FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_account_links FROM PUBLIC;
-- Runtime: schema USAGE; oauth_states SELECT, INSERT(issuer,subject,user_id,revoked_at),
-- UPDATE(revision,revoked_at); bind_flows SELECT,DELETE,
-- INSERT(state_digest,issuer,binding_digest,user_id,user_version,sid).
