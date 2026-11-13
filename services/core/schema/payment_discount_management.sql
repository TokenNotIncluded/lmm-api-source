-- Additive management state. No existing coupon, intent or reservation is removed.
CREATE TABLE core_payments.discount_code_tombstones (
 code_id BIGINT PRIMARY KEY REFERENCES core_payments.discount_codes(id),
 deleted_by BIGINT NOT NULL REFERENCES core_identity.users(id),
 deleted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER discount_code_tombstones_immutable BEFORE UPDATE OR DELETE ON core_payments.discount_code_tombstones
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER discount_code_tombstones_no_truncate BEFORE TRUNCATE ON core_payments.discount_code_tombstones
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TABLE core_payments.discount_management_batches (
 actor_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 request_key UUID NOT NULL,
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 result JSONB NOT NULL CHECK(jsonb_typeof(result)='array'),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(actor_id,request_key)
);
CREATE TRIGGER discount_management_batches_immutable BEFORE UPDATE OR DELETE ON core_payments.discount_management_batches
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER discount_management_batches_no_truncate BEFORE TRUNCATE ON core_payments.discount_management_batches
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_payments.discount_code_tombstones,core_payments.discount_management_batches FROM PUBLIC;
CREATE TABLE core_meta.payment_discount_management_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER payment_discount_management_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.payment_discount_management_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER payment_discount_management_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.payment_discount_management_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.payment_discount_management_upgrades FROM PUBLIC;
