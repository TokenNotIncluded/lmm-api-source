-- OIDC metadata is separate from the frozen native OAuth scope contract.
-- Installed only by the explicit Core installer, never at service startup.
-- Association is written in the same grant transaction. Cross-schema foreign keys
-- would mutate the frozen core_oauth catalog by adding referenced-table triggers.
CREATE SCHEMA core_oidc;
REVOKE ALL ON SCHEMA core_oidc FROM PUBLIC;
CREATE TABLE core_oidc.authorization_requests (
 binding_digest BYTEA PRIMARY KEY CHECK(octet_length(binding_digest)=32),
 nonce TEXT CHECK(octet_length(nonce) BETWEEN 1 AND 256 AND nonce !~ '[[:cntrl:]]')
);
CREATE TABLE core_oidc.families (
 family_id TEXT PRIMARY KEY CHECK(family_id ~ '^lmm_gr_[0-9a-f]{64}$'),
 nonce TEXT,
 auth_time BIGINT NOT NULL CHECK(auth_time>0)
);
CREATE TABLE core_oidc.tokens (
 access_digest BYTEA PRIMARY KEY CHECK(octet_length(access_digest)=32),
 refresh_digest BYTEA NOT NULL UNIQUE CHECK(octet_length(refresh_digest)=32),
 family_id TEXT NOT NULL REFERENCES core_oidc.families(family_id)
);
REVOKE ALL ON ALL TABLES IN SCHEMA core_oidc FROM PUBLIC;

CREATE TABLE core_meta.oauth2_oidc_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER oauth2_oidc_upgrades_immutable BEFORE UPDATE OR DELETE
 ON core_meta.oauth2_oidc_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER oauth2_oidc_upgrades_no_truncate BEFORE TRUNCATE
 ON core_meta.oauth2_oidc_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.oauth2_oidc_upgrades FROM PUBLIC;
