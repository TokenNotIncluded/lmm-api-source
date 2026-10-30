-- Append-only Coinbase support. Historical payment component bytes stay fixed.
-- Existing databases apply this through the explicit offline upgrade command.
ALTER TABLE core_payments.intents
    DROP CONSTRAINT intents_provider_check,
    ADD CONSTRAINT intents_provider_check
        CHECK (provider IN ('stripe','epay','creem','waffo','waffo_pancake','coinbase'));

CREATE TABLE core_meta.coinbase_payment_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK (jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK (octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER coinbase_payment_upgrades_immutable
    BEFORE UPDATE OR DELETE ON core_meta.coinbase_payment_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER coinbase_payment_upgrades_no_truncate
    BEFORE TRUNCATE ON core_meta.coinbase_payment_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.coinbase_payment_upgrades FROM PUBLIC;
