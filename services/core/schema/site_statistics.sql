-- Read-only financial observations. Install/upgrade offline as the schema owner.
-- No ledger rights, balances, credentials, callbacks or refund authority here.
CREATE TABLE core_history.finance_sources (
    source_key TEXT PRIMARY KEY CHECK (source_key ~ '^[a-f0-9]{64}$'),
    contract TEXT NOT NULL CHECK (contract = 'core.site-finance-source.v1'),
    source_archive_sha256 TEXT NOT NULL CHECK (source_archive_sha256 ~ '^[a-f0-9]{64}$'),
    mapping_sha256 TEXT NOT NULL CHECK (mapping_sha256 ~ '^[a-f0-9]{64}$'),
    tables_sha256 TEXT NOT NULL CHECK (tables_sha256 ~ '^[a-f0-9]{64}$'),
    normalization_sha256 TEXT NOT NULL CHECK (normalization_sha256 ~ '^[a-f0-9]{64}$'),
    rows_sha256 TEXT NOT NULL CHECK (rows_sha256 ~ '^[a-f0-9]{64}$'),
    observed_at BIGINT NOT NULL CHECK (observed_at BETWEEN 1 AND 253402300799),
    user_count BIGINT NOT NULL CHECK (user_count >= 0),
    topup_count BIGINT NOT NULL CHECK (topup_count >= 0),
    credits_per_usd BIGINT NOT NULL CHECK (credits_per_usd = 500000),
    complete BOOLEAN NOT NULL CHECK (complete),
    -- The archive has already been read before this seal is inserted. Under
    -- the offline writer fence, all earlier target payment IDs are excluded.
    native_intent_floor BIGINT NOT NULL DEFAULT COALESCE(
        pg_catalog.pg_sequence_last_value('core_payments.intents_id_seq'::regclass),0
    ) CHECK (native_intent_floor >= 0),
    -- Request IDs are opaque strings, so the canonical importer captures the
    -- exact existing set rather than guessing from an authorization timestamp.
    native_request_ids TEXT[] NOT NULL CHECK (array_position(native_request_ids,NULL) IS NULL)
);
-- One complete historical namespace. Multiple archives cannot prove that the
-- same original payment was not counted twice under different source keys.
CREATE UNIQUE INDEX finance_sources_one_archive ON core_history.finance_sources ((true));

CREATE TABLE core_history.finance_users (
    user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL UNIQUE REFERENCES core_billing.account_opening_counters(account_id),
    source_key TEXT NOT NULL REFERENCES core_history.finance_sources(source_key) DEFERRABLE INITIALLY DEFERRED,
    source_row_sha256 TEXT NOT NULL CHECK (source_row_sha256 ~ '^[a-f0-9]{64}$'),
    raw_used_quota BIGINT NOT NULL CHECK (raw_used_quota >= 0),
    normalized_used_credits BIGINT NOT NULL CHECK (normalized_used_credits >= 0)
);

CREATE TABLE core_history.finance_topups (
    source_key TEXT NOT NULL REFERENCES core_history.finance_sources(source_key) DEFERRABLE INITIALLY DEFERRED,
    source_order_id BIGINT NOT NULL CHECK (source_order_id > 0),
    source_row_sha256 TEXT NOT NULL CHECK (source_row_sha256 ~ '^[a-f0-9]{64}$'),
    classification TEXT NOT NULL CHECK (classification IN ('cash','virtual','unconfirmed','invalid','excluded')),
    currency TEXT,
    gross_amount_micros BIGINT,
    refunded_amount_micros BIGINT,
    PRIMARY KEY (source_key,source_order_id),
    CHECK (
        (classification IN ('cash','virtual') AND currency IS NOT NULL
         AND gross_amount_micros IS NOT NULL AND refunded_amount_micros IS NOT NULL
         AND currency ~ '^[A-Z]{3}$'
         AND gross_amount_micros > 0 AND refunded_amount_micros >= 0
         AND refunded_amount_micros <= gross_amount_micros
         AND ((classification='virtual') = (currency='LDC')))
        OR (classification IN ('unconfirmed','invalid','excluded')
            AND currency IS NULL AND gross_amount_micros IS NULL AND refunded_amount_micros IS NULL)
    )
);

CREATE TRIGGER finance_sources_immutable BEFORE UPDATE OR DELETE ON core_history.finance_sources
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER finance_sources_no_truncate BEFORE TRUNCATE ON core_history.finance_sources
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER finance_users_immutable BEFORE UPDATE OR DELETE ON core_history.finance_users
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER finance_users_no_truncate BEFORE TRUNCATE ON core_history.finance_users
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER finance_topups_immutable BEFORE UPDATE OR DELETE ON core_history.finance_topups
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER finance_topups_no_truncate BEFORE TRUNCATE ON core_history.finance_topups
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();

-- Owner-backed aggregate: readers need no access to provider payloads, wire
-- receipts, transaction identifiers or new privileges on existing payment rows.
CREATE VIEW core_history.finance_native_payments WITH (security_barrier=true) AS
WITH integrity AS (
 SELECT NOT EXISTS (
  SELECT 1 FROM pg_catalog.pg_sequence q
  WHERE q.seqrelid='core_payments.intents_id_seq'::regclass
    AND (q.seqincrement<>1 OR q.seqcycle)
 ) AND NOT EXISTS (
  SELECT 1 FROM core_payments.accepted_proofs p
  JOIN core_payments.intents i ON i.id=p.intent_id
  WHERE i.credit_journal_id IS NULL AND p.verified_facts->>'kind' IN ('Credit','RefundSucceeded')
 ) AND NOT EXISTS (
  SELECT 1 FROM core_payments.accepted_proofs p
  JOIN core_payments.intents i ON i.id=p.intent_id
  LEFT JOIN core_payments.provider_refunds r ON r.channel_id=p.channel_id
    AND r.refund_id=p.verified_facts->>'refund_id'
  WHERE i.credit_journal_id IS NOT NULL AND p.verified_facts->>'kind'='RefundSucceeded'
    AND (r.refund_id IS NULL OR r.intent_id<>i.id
      OR r.amount_minor::text<>p.verified_facts->>'amount_minor')
 ) AND NOT EXISTS (
  SELECT 1 FROM core_payments.intents i WHERE i.credit_journal_id IS NOT NULL
   AND i.refunded_minor::numeric<>(SELECT COALESCE(sum(r.amount_minor::numeric),0)
     FROM core_payments.provider_refunds r WHERE r.intent_id=i.id)
 ) AND NOT EXISTS (
  -- observed_at is a dump-start observation, not a snapshot upper bound.
  -- The target sequence seal excludes intents already present during import.
  -- New IDs are disjoint even if the archive and order share a UTC second.
  SELECT 1 FROM core_history.finance_sources s CROSS JOIN core_payments.intents i
  WHERE i.credit_journal_id IS NOT NULL AND i.id<=s.native_intent_floor
 ) AS complete
), totals AS (
 SELECT currency,sum(amount_minor::numeric*10000) AS gross_amount_micros,
   sum(refunded_minor::numeric*10000) AS refunded_amount_micros,count(*)::bigint AS orders
 FROM core_payments.intents WHERE credit_journal_id IS NOT NULL GROUP BY currency
)
SELECT t.currency,t.gross_amount_micros,t.refunded_amount_micros,t.orders,c.complete
FROM integrity c LEFT JOIN totals t ON TRUE;

CREATE TABLE core_meta.site_finance_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK (jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK (octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER site_finance_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.site_finance_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER site_finance_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.site_finance_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();

REVOKE ALL ON core_history.finance_sources, core_history.finance_users,
    core_history.finance_topups, core_history.finance_native_payments,
    core_meta.site_finance_upgrades FROM PUBLIC;
