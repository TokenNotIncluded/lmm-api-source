-- Explicit payment-gateways component. The previous payments component is
-- preserved byte-for-byte; normal startup never executes this delta.
ALTER TABLE core_payments.intents
    DROP CONSTRAINT intents_provider_check,
    DROP CONSTRAINT intents_currency_check,
    DROP CONSTRAINT intents_credits_per_minor_check,
    ADD CONSTRAINT intents_provider_check
        CHECK (provider IN ('stripe','epay','creem','waffo','waffo_pancake')),
    ADD CONSTRAINT intents_currency_check CHECK (currency ~ '^[A-Z]{3}$'),
    ADD CONSTRAINT intents_credits_per_minor_check CHECK (credits_per_minor > 0),
    ADD CONSTRAINT intents_provider_transaction_owner
        UNIQUE (provider,merchant,environment,provider_transaction);

-- Installation and upgrade evidence is not a runtime payment authority.
CREATE TABLE core_meta.payment_gateway_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK (jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK (octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER payment_gateway_upgrades_immutable
    BEFORE UPDATE OR DELETE ON core_meta.payment_gateway_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER payment_gateway_upgrades_no_truncate
    BEFORE TRUNCATE ON core_meta.payment_gateway_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.payment_gateway_upgrades FROM PUBLIC;
