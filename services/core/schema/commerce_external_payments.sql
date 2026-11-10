-- STORE02 external goods payments. Separate from top-ups and Core62 components.
-- Not registered: root coordinates the next component/target and runtime ACL.
CREATE SCHEMA core_commerce_external;

-- Owner-installed merchant/channel association; runtime can only read it.
-- A configured top-up channel does not by itself authorize a merchant's shop.
CREATE TABLE core_commerce_external.merchant_channels (
    seller_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    channel_id BIGINT NOT NULL CHECK(channel_id>0),
    method TEXT NOT NULL CHECK(method IN ('external:epay','external:waffo_pancake','platform:waffo_pancake','platform:linuxdo')),
    scope_sha256 BYTEA NOT NULL CHECK(octet_length(scope_sha256)=32),
    active BOOLEAN NOT NULL DEFAULT true,
    PRIMARY KEY(seller_account_id,channel_id,method)
);
CREATE TABLE core_commerce_external.orders (
    id TEXT PRIMARY KEY CHECK(id ~ '^[a-f0-9]{64}$'),
    shop_id TEXT NOT NULL CHECK(shop_id ~ '^[a-f0-9]{64}$'),
    order_id TEXT NOT NULL CHECK(order_id ~ '^[a-f0-9]{64}$'),
    service_principal TEXT NOT NULL,
    command_sha256 BYTEA NOT NULL CHECK(octet_length(command_sha256)=32),
    command JSONB NOT NULL,
    channel_scope JSONB NOT NULL,
    buyer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    seller_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    provider_order_reference TEXT NOT NULL UNIQUE,
    provider_buyer_reference TEXT NOT NULL,
    quantity BIGINT NOT NULL CHECK(quantity>0),
    principal_credits BIGINT NOT NULL CHECK(principal_credits>0),
    quoted_minor BIGINT NOT NULL CHECK(quoted_minor>0),
    currency TEXT NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
    UNIQUE(shop_id,order_id)
);
CREATE TABLE core_commerce_external.payment_bases (
    order_id TEXT PRIMARY KEY REFERENCES core_commerce_external.orders(id),
    id TEXT NOT NULL UNIQUE CHECK(id ~ '^[a-f0-9]{64}$'),
    provider TEXT NOT NULL,
    merchant TEXT NOT NULL,
    environment TEXT NOT NULL,
    provider_order_reference TEXT NOT NULL,
    receipt_reference TEXT NOT NULL,
    payment_reference TEXT NOT NULL,
    charged_minor BIGINT NOT NULL CHECK(charged_minor>0),
    currency TEXT NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
    facts_sha256 BYTEA NOT NULL CHECK(octet_length(facts_sha256)=32),
    payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
    -- Signed raw evidence stays in Core. Never returned as public order JSON.
    signed_payload BYTEA NOT NULL CHECK(octet_length(signed_payload) BETWEEN 1 AND 32768),
    signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
    UNIQUE(provider,merchant,environment,payment_reference),
    UNIQUE(provider,merchant,environment,receipt_reference)
);
CREATE TABLE core_commerce_external.refunds (
    id TEXT PRIMARY KEY CHECK(id ~ '^[a-f0-9]{64}$'),
    order_id TEXT NOT NULL REFERENCES core_commerce_external.orders(id),
    basis_id TEXT NOT NULL REFERENCES core_commerce_external.payment_bases(id),
    command_sha256 BYTEA NOT NULL CHECK(octet_length(command_sha256)=32),
    command JSONB NOT NULL,
    authorized_by BIGINT NOT NULL REFERENCES core_identity.users(id),
    mode TEXT NOT NULL CHECK(mode IN ('full','quantity','amount')),
    quantity BIGINT NOT NULL CHECK(quantity>=0),
    retired_quantity BIGINT NOT NULL CHECK(retired_quantity>=0),
    credits BIGINT NOT NULL CHECK(credits>0),
    native_minor BIGINT NOT NULL CHECK(native_minor>0),
    state TEXT NOT NULL CHECK(state IN ('approved','unknown','executed','reconciliation_required','completed','rejected','cancelled','manual')),
    submit_count SMALLINT NOT NULL DEFAULT 0 CHECK(submit_count IN (0,1)),
    dispatch_fence TEXT UNIQUE,
    provider_execution_receipt TEXT,
    settlement_receipt TEXT,
    CHECK((submit_count=0)=(dispatch_fence IS NULL)),
    CHECK(state<>'cancelled' OR submit_count=0),
    CHECK(state NOT IN ('executed','reconciliation_required','completed') OR submit_count=1),
    CHECK(state<>'completed' OR (provider_execution_receipt IS NOT NULL AND settlement_receipt IS NOT NULL))
);

CREATE FUNCTION core_commerce_external.immutable() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'immutable external commerce identity' USING ERRCODE='55000'; END $$;
CREATE TRIGGER orders_immutable BEFORE UPDATE OR DELETE ON core_commerce_external.orders FOR EACH ROW EXECUTE FUNCTION core_commerce_external.immutable();
CREATE TRIGGER bases_immutable BEFORE UPDATE OR DELETE ON core_commerce_external.payment_bases FOR EACH ROW EXECUTE FUNCTION core_commerce_external.immutable();
CREATE TRIGGER orders_no_truncate BEFORE TRUNCATE ON core_commerce_external.orders FOR EACH STATEMENT EXECUTE FUNCTION core_commerce_external.immutable();
CREATE TRIGGER bases_no_truncate BEFORE TRUNCATE ON core_commerce_external.payment_bases FOR EACH STATEMENT EXECUTE FUNCTION core_commerce_external.immutable();
CREATE FUNCTION core_commerce_external.guard_refund() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP<>'UPDATE' THEN RAISE EXCEPTION 'external refund cannot be deleted' USING ERRCODE='55000'; END IF;
    IF (to_jsonb(NEW)-ARRAY['state','submit_count','dispatch_fence','provider_execution_receipt','settlement_receipt'])
       IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','submit_count','dispatch_fence','provider_execution_receipt','settlement_receipt'])
       OR NEW.submit_count<OLD.submit_count
       OR (OLD.dispatch_fence IS NOT NULL AND NEW.dispatch_fence IS DISTINCT FROM OLD.dispatch_fence)
       OR (OLD.provider_execution_receipt IS NOT NULL AND NEW.provider_execution_receipt IS DISTINCT FROM OLD.provider_execution_receipt)
       OR (OLD.settlement_receipt IS NOT NULL AND NEW.settlement_receipt IS DISTINCT FROM OLD.settlement_receipt)
       OR (OLD.state IN ('completed','rejected','cancelled') AND NEW IS DISTINCT FROM OLD)
       OR (NEW.submit_count>OLD.submit_count AND (OLD.state<>'approved' OR NEW.state<>'unknown'))
       OR (OLD.submit_count=1 AND NEW.state IN ('approved','cancelled','manual')) THEN
        RAISE EXCEPTION 'external refund identity or dispatch fence changed' USING ERRCODE='55000';
    END IF;
    IF NEW.state IN ('executed','reconciliation_required','completed') AND NEW.provider_execution_receipt IS NULL
       OR (OLD.submit_count=1 AND NEW.state='rejected' AND (NEW.provider_execution_receipt IS NULL OR NEW.settlement_receipt IS NULL)) THEN
        RAISE EXCEPTION 'external refund has no final execution authority' USING ERRCODE='55000';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER refund_identity BEFORE UPDATE OR DELETE ON core_commerce_external.refunds FOR EACH ROW EXECUTE FUNCTION core_commerce_external.guard_refund();
CREATE TRIGGER refunds_no_truncate BEFORE TRUNCATE ON core_commerce_external.refunds FOR EACH STATEMENT EXECUTE FUNCTION core_commerce_external.immutable();
REVOKE ALL ON SCHEMA core_commerce_external FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_commerce_external FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_commerce_external FROM PUBLIC;
