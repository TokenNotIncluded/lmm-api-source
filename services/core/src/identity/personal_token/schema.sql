-- Explicit installer component; never executed by token HTTP handlers.
CREATE SCHEMA core_personal_tokens;
CREATE TABLE core_personal_tokens.tokens (
    user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
    digest BYTEA NOT NULL UNIQUE CHECK (octet_length(digest)=32),
    user_version BIGINT NOT NULL CHECK (user_version>0),
    generation BIGINT NOT NULL CHECK (generation>0),
    expires_at TIMESTAMPTZ NOT NULL
);
-- Runtime requires USAGE on schema and SELECT, INSERT, UPDATE on tokens.
-- Old plaintext personal tokens are never imported or promoted to Sessions.

CREATE TABLE core_meta.personal_tokens_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER personal_tokens_upgrades_immutable BEFORE UPDATE OR DELETE
 ON core_meta.personal_tokens_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER personal_tokens_upgrades_no_truncate BEFORE TRUNCATE
 ON core_meta.personal_tokens_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.personal_tokens_upgrades FROM PUBLIC;
