-- Native acquisition attribution; does not mint or replay financial rewards.
CREATE TABLE core_identity.affiliate_codes (
 user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
 code TEXT NOT NULL UNIQUE CHECK(code ~ '^[a-f0-9]{32}$')
);
CREATE TABLE core_identity.affiliate_registrations (
 user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
 inviter_id BIGINT NOT NULL REFERENCES core_identity.affiliate_codes(user_id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK(user_id <> inviter_id)
);
REVOKE ALL ON core_identity.affiliate_codes, core_identity.affiliate_registrations FROM PUBLIC;
CREATE TRIGGER affiliate_code_immutable BEFORE UPDATE OR DELETE ON core_identity.affiliate_codes
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER affiliate_registration_immutable BEFORE UPDATE OR DELETE ON core_identity.affiliate_registrations
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();

CREATE TABLE core_meta.affiliate_registration_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER affiliate_registration_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.affiliate_registration_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER affiliate_registration_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.affiliate_registration_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.affiliate_registration_upgrades FROM PUBLIC;
