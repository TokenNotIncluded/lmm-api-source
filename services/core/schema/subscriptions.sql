-- Explicit fresh installation only, after charging-ledger. Runtime never runs DDL.
-- Sales use the canonical Core ledger; grants remain in core_charging.
CREATE SCHEMA core_subscriptions;
CREATE TABLE core_subscriptions.plans (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    revision BIGINT NOT NULL CHECK (revision > 0),
    archived BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE TABLE core_subscriptions.plan_revisions (
    plan_id BIGINT NOT NULL REFERENCES core_subscriptions.plans(id),
    revision BIGINT NOT NULL CHECK (revision > 0),
    terms JSONB NOT NULL CHECK (jsonb_typeof(terms) = 'object'),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    created_at BIGINT NOT NULL,
    PRIMARY KEY (plan_id, revision)
);
ALTER TABLE core_subscriptions.plans ADD FOREIGN KEY (id,revision)
    REFERENCES core_subscriptions.plan_revisions(plan_id,revision) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE core_subscriptions.commands (
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    operation_key TEXT NOT NULL CHECK (octet_length(operation_key) BETWEEN 16 AND 80),
    request JSONB NOT NULL,
    result JSONB NOT NULL,
    PRIMARY KEY (actor_user_id, operation_key)
);
CREATE TABLE core_subscriptions.quotes (
    id TEXT PRIMARY KEY CHECK (id ~ '^[0-9a-f]{64}$'),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    operation_key TEXT NOT NULL CHECK (octet_length(operation_key) BETWEEN 16 AND 80),
    plan_id BIGINT NOT NULL,
    plan_revision BIGINT NOT NULL,
    renew_from BIGINT,
    starts_at BIGINT NOT NULL,
    ends_at BIGINT NOT NULL CHECK (ends_at > starts_at),
    expires_at BIGINT NOT NULL,
    state TEXT NOT NULL DEFAULT 'quoted' CHECK (state IN ('quoted','posted','rejected')),
    result JSONB,
    CHECK ((state='quoted')=(result IS NULL)),
    UNIQUE (actor_user_id,operation_key),
    FOREIGN KEY (plan_id,plan_revision) REFERENCES core_subscriptions.plan_revisions(plan_id,revision)
);
CREATE TABLE core_subscriptions.purchases (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    quote_id TEXT NOT NULL UNIQUE REFERENCES core_subscriptions.quotes(id),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    subscription_id BIGINT NOT NULL UNIQUE REFERENCES core_charging.subscriptions(id),
    charge_journal_id BIGINT NOT NULL UNIQUE REFERENCES core_billing.ledger_journals(id),
    canceled_at BIGINT,
    refund_journal_id BIGINT UNIQUE REFERENCES core_billing.ledger_journals(id)
);
ALTER TABLE core_subscriptions.quotes ADD FOREIGN KEY (renew_from) REFERENCES core_subscriptions.purchases(id);
CREATE UNIQUE INDEX one_successful_renewal ON core_subscriptions.quotes(renew_from)
    WHERE renew_from IS NOT NULL AND state='posted';
CREATE INDEX purchases_owner ON core_subscriptions.purchases(actor_user_id,id);
CREATE TABLE core_subscriptions.refund_quotes (
    id TEXT PRIMARY KEY CHECK (id ~ '^[0-9a-f]{64}$'),
    purchase_id BIGINT NOT NULL REFERENCES core_subscriptions.purchases(id),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    expires_at BIGINT NOT NULL,
    result JSONB
);
CREATE FUNCTION core_subscriptions.guard_mutation() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'subscription history is immutable' USING ERRCODE='23514'; END IF;
    IF TG_TABLE_NAME='purchases' THEN
        IF (to_jsonb(NEW)-ARRAY['canceled_at','refund_journal_id']) IS DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['canceled_at','refund_journal_id']) OR
           (OLD.canceled_at IS NOT NULL AND NEW.canceled_at IS DISTINCT FROM OLD.canceled_at) OR
           (OLD.refund_journal_id IS NOT NULL AND NEW.refund_journal_id IS DISTINCT FROM OLD.refund_journal_id) THEN
            RAISE EXCEPTION 'immutable subscription purchase' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='quotes' THEN
        IF (to_jsonb(NEW)-ARRAY['state','result']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','result']) OR OLD.state<>'quoted' THEN
            RAISE EXCEPTION 'immutable subscription quote' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='refund_quotes' THEN
        IF (to_jsonb(NEW)-'result') IS DISTINCT FROM (to_jsonb(OLD)-'result') OR OLD.result IS NOT NULL THEN
            RAISE EXCEPTION 'immutable subscription refund' USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER purchase_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.purchases
    FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_mutation();
CREATE TRIGGER quote_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.quotes
    FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_mutation();
CREATE TRIGGER refund_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.refund_quotes
    FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_mutation();
CREATE TRIGGER revisions_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.plan_revisions
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER commands_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.commands
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
-- A durable sale must bind one actual wallet charge to one exact grant. These
-- deferred constraints observe the complete financial transaction at COMMIT.
CREATE FUNCTION core_subscriptions.check_purchase() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE p core_subscriptions.purchases%ROWTYPE; q core_subscriptions.quotes%ROWTYPE;
    terms JSONB; s core_charging.subscriptions%ROWTYPE; j core_billing.ledger_journals%ROWTYPE;
BEGIN
    SELECT * INTO STRICT p FROM core_subscriptions.purchases WHERE id=NEW.id;
    SELECT * INTO STRICT q FROM core_subscriptions.quotes WHERE id=p.quote_id;
    SELECT r.terms INTO STRICT terms FROM core_subscriptions.plan_revisions r WHERE r.plan_id=q.plan_id AND r.revision=q.plan_revision;
    SELECT * INTO STRICT s FROM core_charging.subscriptions WHERE id=p.subscription_id;
    SELECT * INTO STRICT j FROM core_billing.ledger_journals WHERE id=p.charge_journal_id;
    IF p.actor_user_id<>q.actor_user_id OR p.account_id<>q.account_id OR q.state<>'posted'
       OR s.account_id<>p.account_id OR s.grant_reference<>'subscription:'||q.id
       OR s.anchor<>q.starts_at OR s.ends_at<>q.ends_at OR s.period<>terms->>'reset_period'
       OR s.seconds<>(terms->>'reset_seconds')::bigint OR s.limit_credits<>(terms->>'limit_units')::bigint
       OR s.unlimited OR s.reset_basis<>'same' OR to_jsonb(s.models)<>terms->'models' OR to_jsonb(s.groups)<>terms->'groups'
       OR j.kind<>'charge' OR j.scope<>'core-subscription-wallet' OR j.operation_key<>'buy:'||q.id
       OR j.amount_units<>(terms->>'price_units')::bigint OR j.actor_user_id IS DISTINCT FROM p.actor_user_id
       OR NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts a WHERE a.id=j.payer_id AND a.owner_account_id=p.account_id AND a.bucket='wallet') THEN
        RAISE EXCEPTION 'subscription purchase does not match grant and charge' USING ERRCODE='23514';
    END IF;
    IF p.refund_journal_id IS NOT NULL THEN
        SELECT * INTO STRICT j FROM core_billing.ledger_journals WHERE id=p.refund_journal_id;
        IF s.active OR p.canceled_at IS NULL OR j.kind<>'refund' OR j.scope<>'core-subscription-wallet'
           OR j.parent_journal_id<>p.charge_journal_id OR j.amount_units<>(terms->>'price_units')::bigint
           OR j.actor_user_id IS DISTINCT FROM p.actor_user_id THEN
            RAISE EXCEPTION 'subscription refund does not match original sale' USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER purchase_bound AFTER INSERT OR UPDATE ON core_subscriptions.purchases
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_subscriptions.check_purchase();
REVOKE ALL ON SCHEMA core_subscriptions FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_subscriptions FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_subscriptions FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_subscriptions FROM PUBLIC;
