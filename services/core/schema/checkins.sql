-- Source-compatible check-ins. Historical awards are already in opening balances.
CREATE SCHEMA core_checkins;
CREATE TABLE core_checkins.seed_receipts (
    seed_id TEXT PRIMARY KEY,
    history_migration_id TEXT NOT NULL UNIQUE REFERENCES core_migration.import_runs(migration_id),
    identity_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id),
    source_archive_sha256 TEXT NOT NULL CHECK(source_archive_sha256 ~ '^[0-9a-f]{64}$'),
    mapping_sha256 TEXT NOT NULL CHECK(mapping_sha256 ~ '^[0-9a-f]{64}$'),
    source_cutoff BIGINT NOT NULL CHECK(source_cutoff>0),
    source_options_sha256 TEXT NOT NULL CHECK(source_options_sha256 ~ '^[0-9a-f]{64}$'),
    settings JSONB NOT NULL CHECK(jsonb_typeof(settings)='object'),
    result JSONB NOT NULL
);
CREATE TABLE core_checkins.policy_revisions (
    revision BIGINT PRIMARY KEY CHECK(revision>0),
    policy JSONB NOT NULL CHECK(jsonb_typeof(policy)='object'),
    source_options JSONB NOT NULL CHECK(jsonb_typeof(source_options)='object'),
    seed_id TEXT NOT NULL REFERENCES core_checkins.seed_receipts(seed_id),
    created_at BIGINT NOT NULL CHECK(created_at>=0)
);
CREATE TABLE core_checkins.active_policy (
    singleton BOOLEAN PRIMARY KEY CHECK(singleton),
    revision BIGINT NOT NULL REFERENCES core_checkins.policy_revisions(revision)
);
CREATE TABLE core_checkins.owner_commands (
    operation_key TEXT PRIMARY KEY CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
    request JSONB NOT NULL,
    result JSONB NOT NULL
);
CREATE TABLE core_checkins.days (
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    checkin_date DATE NOT NULL,
    amount_units BIGINT NOT NULL CHECK(amount_units>=0),
    origin TEXT NOT NULL CHECK(origin IN ('native','history')),
    policy_revision BIGINT REFERENCES core_checkins.policy_revisions(revision),
    seed_id TEXT REFERENCES core_checkins.seed_receipts(seed_id),
    journal_id BIGINT UNIQUE REFERENCES core_billing.ledger_journals(id),
    created_at BIGINT NOT NULL CHECK(created_at>=0),
    result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
    PRIMARY KEY(account_id,checkin_date),
    UNIQUE(user_id,checkin_date),
    CHECK((origin='native' AND policy_revision IS NOT NULL AND seed_id IS NULL
          AND ((amount_units=0 AND journal_id IS NULL) OR (amount_units>0 AND journal_id IS NOT NULL)))
       OR (origin='history' AND policy_revision IS NULL AND seed_id IS NOT NULL AND journal_id IS NULL))
);
CREATE TABLE core_checkins.claim_commands (
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
    result JSONB NOT NULL,
    PRIMARY KEY(user_id,operation_key)
);
CREATE FUNCTION core_checkins.guard_day() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF NOT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a
      ON a.id=u.personal_account_id WHERE u.id=NEW.user_id
      AND u.personal_account_id=NEW.account_id AND a.kind='personal') THEN
        RAISE EXCEPTION 'checkin personal account mismatch' USING ERRCODE='23514';
    END IF;
    IF jsonb_typeof(NEW.result) IS DISTINCT FROM 'object'
      OR (NEW.result->>'checkin_date') IS DISTINCT FROM NEW.checkin_date::text
      OR (NEW.result->>'quota_awarded') IS DISTINCT FROM NEW.amount_units::text
      OR (NEW.result->>'origin') IS DISTINCT FROM NEW.origin
      OR (NEW.result->>'created_at')::bigint IS DISTINCT FROM NEW.created_at
      OR (NEW.result->>'journal_id')::bigint IS DISTINCT FROM NEW.journal_id
      OR (NEW.result->>'policy_revision')::bigint IS DISTINCT FROM NEW.policy_revision THEN
        RAISE EXCEPTION 'checkin receipt mismatch' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER day_binding BEFORE INSERT ON core_checkins.days
FOR EACH ROW EXECUTE FUNCTION core_checkins.guard_day();
CREATE TRIGGER day_immutable BEFORE UPDATE OR DELETE ON core_checkins.days
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER policy_immutable BEFORE UPDATE OR DELETE ON core_checkins.policy_revisions
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER owner_command_immutable BEFORE UPDATE OR DELETE ON core_checkins.owner_commands
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER claim_command_immutable BEFORE UPDATE OR DELETE ON core_checkins.claim_commands
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER seed_immutable BEFORE UPDATE OR DELETE ON core_checkins.seed_receipts
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER day_no_truncate BEFORE TRUNCATE ON core_checkins.days
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER policy_no_truncate BEFORE TRUNCATE ON core_checkins.policy_revisions
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER owner_command_no_truncate BEFORE TRUNCATE ON core_checkins.owner_commands
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER claim_command_no_truncate BEFORE TRUNCATE ON core_checkins.claim_commands
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER seed_no_truncate BEFORE TRUNCATE ON core_checkins.seed_receipts
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON SCHEMA core_checkins FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_checkins FROM PUBLIC;
