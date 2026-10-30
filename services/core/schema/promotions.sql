-- Explicit fresh install after ledger/identity/abuse. Core owns this budget and
-- every gift posting; extension campaign records are not monetary authority.
CREATE SCHEMA core_promotions;
CREATE TABLE core_promotions.secrets (
    singleton BOOLEAN PRIMARY KEY CHECK(singleton),
    code_key BYTEA NOT NULL CHECK(octet_length(code_key)=32)
);
INSERT INTO core_promotions.secrets VALUES(TRUE,decode(replace(gen_random_uuid()::text||gen_random_uuid()::text,'-',''),'hex'));
CREATE TABLE core_promotions.campaigns (
    id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
    revision BIGINT NOT NULL CHECK(revision>0),
    kind TEXT NOT NULL CHECK(kind IN ('redemption','checkin','envelope')),
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 160),
    award_units BIGINT NOT NULL CHECK(award_units BETWEEN 1 AND 1000000000000),
    budget_units BIGINT NOT NULL CHECK(budget_units BETWEEN award_units AND 1000000000000),
    spent_units BIGINT NOT NULL DEFAULT 0 CHECK(spent_units>=0),
    reserved_units BIGINT NOT NULL DEFAULT 0 CHECK(reserved_units>=0),
    max_claims BIGINT NOT NULL CHECK(max_claims BETWEEN 1 AND 1000000),
    claims BIGINT NOT NULL DEFAULT 0 CHECK(claims>=0),
    reserved_claims BIGINT NOT NULL DEFAULT 0 CHECK(reserved_claims>=0),
    max_claims_per_user BIGINT NOT NULL CHECK(max_claims_per_user BETWEEN 1 AND 10000),
    starts_at BIGINT NOT NULL CHECK(starts_at>=0),
    ends_at BIGINT NOT NULL CHECK(ends_at>starts_at AND ends_at<=253402300799),
    enabled BOOLEAN NOT NULL,
    CHECK(spent_units+reserved_units<=budget_units),
    CHECK(claims+reserved_claims<=max_claims)
);
CREATE TABLE core_promotions.commands (
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
    request JSONB NOT NULL,
    result JSONB NOT NULL,
    PRIMARY KEY(actor_user_id,operation_key)
);
CREATE TABLE core_promotions.issues (
    id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
    campaign_id TEXT NOT NULL REFERENCES core_promotions.campaigns(id),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    quantity INTEGER NOT NULL CHECK(quantity BETWEEN 1 AND 100),
    created_at BIGINT NOT NULL
);
CREATE TABLE core_promotions.codes (
    digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
    issue_id TEXT NOT NULL REFERENCES core_promotions.issues(id),
    position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 99),
    campaign_id TEXT NOT NULL REFERENCES core_promotions.campaigns(id),
    revoked BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE(issue_id,position)
);
CREATE TABLE core_promotions.claims (
    id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{64}$'),
    campaign_id TEXT NOT NULL REFERENCES core_promotions.campaigns(id),
    subject_key TEXT NOT NULL CHECK(length(subject_key) BETWEEN 1 AND 128),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    amount_units BIGINT NOT NULL CHECK(amount_units>0),
    code_digest BYTEA UNIQUE REFERENCES core_promotions.codes(digest),
    journal_id BIGINT NOT NULL UNIQUE REFERENCES core_billing.ledger_journals(id),
    created_at BIGINT NOT NULL,
    UNIQUE(campaign_id,subject_key)
);
CREATE INDEX claims_actor ON core_promotions.claims(actor_user_id,created_at,id);
CREATE INDEX claims_campaign_actor ON core_promotions.claims(campaign_id,actor_user_id);
CREATE FUNCTION core_promotions.guard_campaign() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='DELETE' OR (NEW.id,NEW.kind,NEW.award_units) IS DISTINCT FROM (OLD.id,OLD.kind,OLD.award_units)
        OR NEW.spent_units<OLD.spent_units OR NEW.claims<OLD.claims OR NEW.revision<OLD.revision THEN
        RAISE EXCEPTION 'immutable campaign award/history' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER campaign_identity BEFORE UPDATE OR DELETE ON core_promotions.campaigns
    FOR EACH ROW EXECUTE FUNCTION core_promotions.guard_campaign();
CREATE FUNCTION core_promotions.guard_code() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='DELETE' OR (to_jsonb(NEW)-'revoked') IS DISTINCT FROM (to_jsonb(OLD)-'revoked') OR OLD.revoked THEN
        RAISE EXCEPTION 'immutable promotion code' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER code_identity BEFORE UPDATE OR DELETE ON core_promotions.codes
    FOR EACH ROW EXECUTE FUNCTION core_promotions.guard_code();
CREATE TRIGGER claim_immutable BEFORE UPDATE OR DELETE ON core_promotions.claims
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER issue_immutable BEFORE UPDATE OR DELETE ON core_promotions.issues
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER command_immutable BEFORE UPDATE OR DELETE ON core_promotions.commands
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE FUNCTION core_promotions.check_claim() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE j core_billing.ledger_journals%ROWTYPE; c core_promotions.campaigns%ROWTYPE;
BEGIN
    SELECT * INTO STRICT j FROM core_billing.ledger_journals WHERE id=NEW.journal_id;
    SELECT * INTO STRICT c FROM core_promotions.campaigns WHERE id=NEW.campaign_id;
    IF j.kind<>'gift' OR j.scope<>'core-promotion-rewards' OR j.operation_key<>'grant:'||NEW.id
       OR j.amount_units<>NEW.amount_units OR NEW.amount_units<>c.award_units
       OR j.actor_user_id IS DISTINCT FROM NEW.actor_user_id
       OR NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts a WHERE a.id=j.payee_id AND a.owner_account_id=NEW.account_id AND a.bucket='wallet')
       OR NOT EXISTS(SELECT 1 FROM core_identity.users u WHERE u.id=NEW.actor_user_id AND u.personal_account_id=NEW.account_id)
       OR (c.kind='redemption') IS DISTINCT FROM (NEW.code_digest IS NOT NULL) THEN
        RAISE EXCEPTION 'promotion claim does not match gift and account' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER claim_bound AFTER INSERT ON core_promotions.claims
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_promotions.check_claim();
REVOKE ALL ON SCHEMA core_promotions FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_promotions FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_promotions FROM PUBLIC;
