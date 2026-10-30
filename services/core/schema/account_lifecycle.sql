-- Fresh component only. Existing installations require a reviewed offline
-- component upgrade. Runtime startup never installs or upgrades this schema.
CREATE SCHEMA core_account_lifecycle;
CREATE TABLE core_account_lifecycle.closures (
    user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL UNIQUE REFERENCES core_identity.accounts(id),
    credential_id BIGINT NOT NULL REFERENCES core_identity.credentials(id),
    previous_auth_version BIGINT NOT NULL CHECK(previous_auth_version>0),
    operation_digest BYTEA NOT NULL UNIQUE CHECK(octet_length(operation_digest)=32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER closure_immutable BEFORE UPDATE OR DELETE ON core_account_lifecycle.closures
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER closure_no_truncate BEFORE TRUNCATE ON core_account_lifecycle.closures
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();

-- Only lock and inspect the existing ledger projection. Runtime receives no
-- direct UPDATE authority on balances, and this function never posts money.
CREATE FUNCTION core_account_lifecycle.wallet_is_empty(p_account_id BIGINT) RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    IF p_account_id IS NULL OR p_account_id<=0 THEN
        RAISE EXCEPTION 'invalid account' USING ERRCODE='23514';
    END IF;
    PERFORM b.ledger_account_id FROM core_billing.balance_state b
        JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id
        WHERE a.owner_account_id=p_account_id ORDER BY b.ledger_account_id FOR UPDATE OF b;
    RETURN NOT EXISTS(SELECT 1 FROM core_billing.balance_state b
        JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id
        WHERE a.owner_account_id=p_account_id AND b.balance_units<>0);
END $$;

CREATE FUNCTION core_account_lifecycle.check_closure() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
        WHERE u.id=NEW.user_id AND a.id=NEW.account_id AND NOT u.active AND NOT a.active
        AND u.auth_version>NEW.previous_auth_version)
        OR EXISTS(SELECT 1 FROM core_identity.credentials WHERE user_id=NEW.user_id AND revoked_at IS NULL)
        OR EXISTS(SELECT 1 FROM core_identity.web_sessions WHERE user_id=NEW.user_id AND revoked_at IS NULL) THEN
        RAISE EXCEPTION 'closure receipt requires terminal authority revocation' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER closure_terminal AFTER INSERT ON core_account_lifecycle.closures
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_account_lifecycle.check_closure();
REVOKE ALL ON SCHEMA core_account_lifecycle FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_account_lifecycle FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_account_lifecycle FROM PUBLIC;
-- Runtime: schema USAGE; closures SELECT, INSERT(user_id,account_id,credential_id,
-- previous_auth_version,operation_digest); EXECUTE wallet_is_empty(bigint).
-- Read-only preflight also needs SELECT on charging.subscriptions,
-- commerce.orders, payments.intents/refund_holds/blocked_refunds and wallet.transfers.
