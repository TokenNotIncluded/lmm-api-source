-- Empty language preserves provider inference. Existing explicit values stay unchanged.
ALTER TABLE core_subscriptions.pancake_prices
 DROP CONSTRAINT pancake_prices_checkout_language_check;
ALTER TABLE core_subscriptions.pancake_prices
 ADD CONSTRAINT pancake_prices_checkout_language_check
 CHECK(length(checkout_language) <= 32);

-- Standard owner-only durable upgrade receipt, identical on both paths.
CREATE TABLE core_meta.pancake_language_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER pancake_language_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.pancake_language_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_language_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.pancake_language_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.pancake_language_upgrades FROM PUBLIC;
