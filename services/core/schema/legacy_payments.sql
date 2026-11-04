-- Fresh component 50. Owner-reviewed frozen terms are not runtime-writable.

CREATE TABLE core_payments.legacy_bases (
 id TEXT PRIMARY KEY CHECK(id ~ '^legacy_[a-f0-9]{64}$'),
 opening_migration_id TEXT NOT NULL,
 source_archive_sha256 TEXT NOT NULL,
 source_order_id BIGINT NOT NULL CHECK(source_order_id>0),
 source_user_id BIGINT NOT NULL CHECK(source_user_id>0),
 opening_entity_key TEXT GENERATED ALWAYS AS ('user:'||source_user_id::text) STORED,
 source_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_row_sha256)=32),
 review_sha256 BYTEA NOT NULL CHECK(octet_length(review_sha256)=32),
 user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 provider TEXT NOT NULL CHECK(provider='waffo_pancake'),
 channel_id BIGINT NOT NULL CHECK(channel_id>0),
 merchant TEXT NOT NULL CHECK(length(merchant) BETWEEN 1 AND 128),
 environment TEXT NOT NULL CHECK(environment IN ('test','live')),
 currency TEXT NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 order_reference TEXT NOT NULL CHECK(order_reference ~ '^[A-Za-z0-9_.-]{1,128}$'),
 store_id TEXT NOT NULL CHECK(length(store_id) BETWEEN 1 AND 128),
 source_product_id TEXT CHECK(length(source_product_id) BETWEEN 1 AND 128),
 amount_micros BIGINT NOT NULL CHECK(amount_micros>0),
 credit_units BIGINT NOT NULL CHECK(credit_units>0),
 discount_code_id BIGINT NOT NULL DEFAULT 0 CHECK(discount_code_id>=0),
 source_status TEXT NOT NULL CHECK(source_status IN ('pending','failed')),
 failure_reason TEXT NOT NULL DEFAULT '',
 CHECK(source_status='pending' OR failure_reason='checkout_timeout'),
 FOREIGN KEY(opening_migration_id,opening_entity_key)
   REFERENCES core_billing.opening_imports(migration_id,source_entity_key),
 UNIQUE(source_archive_sha256,source_order_id),
 UNIQUE(provider,merchant,environment,order_reference),
 UNIQUE(id,provider,merchant,environment)
);

CREATE FUNCTION core_payments.guard_legacy_basis() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NOT EXISTS (
   SELECT FROM core_identity.users u
   JOIN core_billing.opening_imports i ON i.account_id=u.personal_account_id
   JOIN core_billing.opening_import_runs r ON r.migration_id=i.migration_id
   WHERE u.id=NEW.user_id AND u.personal_account_id=NEW.account_id
     AND i.migration_id=NEW.opening_migration_id
     AND i.source_entity_key='user:'||NEW.source_user_id::text
     AND r.source_archive_sha256=NEW.source_archive_sha256
 ) THEN
   RAISE EXCEPTION 'legacy basis must match original opening and personal owner' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER legacy_basis_source BEFORE INSERT ON core_payments.legacy_bases
 FOR EACH ROW EXECUTE FUNCTION core_payments.guard_legacy_basis();

CREATE TABLE core_payments.legacy_proofs (
 id TEXT PRIMARY KEY CHECK(id ~ '^lproof_[a-f0-9]{64}$'),
 basis_id TEXT NOT NULL,
 provider TEXT NOT NULL,
 merchant TEXT NOT NULL,
 environment TEXT NOT NULL,
 event_id TEXT NOT NULL CHECK(length(event_id) BETWEEN 1 AND 128),
 order_id TEXT NOT NULL CHECK(length(order_id) BETWEEN 1 AND 128),
 payment_id TEXT CHECK(length(payment_id) BETWEEN 1 AND 128),
 payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
 signed_payload BYTEA NOT NULL CHECK(octet_length(signed_payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 accepted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(basis_id,provider,merchant,environment)
   REFERENCES core_payments.legacy_bases(id,provider,merchant,environment),
 UNIQUE(provider,merchant,environment,event_id),
 UNIQUE(basis_id,id)
);

-- One namespace for both native and legacy payments, including ORD/PAY aliases.
CREATE TABLE core_payments.provider_identifiers (
 provider TEXT NOT NULL,
 merchant TEXT NOT NULL,
 environment TEXT NOT NULL CHECK(environment IN ('test','live')),
 kind TEXT NOT NULL CHECK(kind IN ('event','order','payment')),
 identifier TEXT NOT NULL CHECK(length(identifier) BETWEEN 1 AND 128),
 native_intent_id BIGINT REFERENCES core_payments.intents(id),
 legacy_basis_id TEXT REFERENCES core_payments.legacy_bases(id),
 CHECK(num_nonnulls(native_intent_id,legacy_basis_id)=1),
 PRIMARY KEY(provider,merchant,environment,kind,identifier)
);

CREATE TABLE core_payments.legacy_applications (
 operation_id TEXT PRIMARY KEY CHECK(operation_id ~ '^lapply_[a-f0-9]{64}$'),
 basis_id TEXT NOT NULL REFERENCES core_payments.legacy_bases(id),
 proof_id TEXT NOT NULL,
 journal_id BIGINT UNIQUE REFERENCES core_billing.ledger_journals(id),
 outcome JSONB NOT NULL CHECK(COALESCE(outcome->>'status' IN ('posted','rejected'),FALSE)),
 CHECK((journal_id IS NOT NULL)=(outcome->>'status'='posted')),
 FOREIGN KEY(basis_id,proof_id) REFERENCES core_payments.legacy_proofs(basis_id,id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX legacy_credit_once ON core_payments.legacy_applications(basis_id)
 WHERE journal_id IS NOT NULL;

-- Each posted legacy discount use consumes its original held order exactly
-- once. New discount admission must subtract these orders from the imported
-- held set and add these uses to the original used count, not double-count both.
CREATE VIEW core_payments.legacy_discount_consumptions WITH(security_invoker=true) AS
 SELECT b.source_archive_sha256,b.source_order_id,b.discount_code_id,a.journal_id
 FROM core_payments.legacy_bases b JOIN core_payments.legacy_applications a ON a.basis_id=b.id
 WHERE a.journal_id IS NOT NULL AND b.discount_code_id>0;

CREATE FUNCTION core_payments.guard_legacy_application() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.journal_id IS NOT NULL AND NOT EXISTS (
   SELECT FROM core_billing.ledger_journals j
   JOIN core_billing.ledger_accounts a ON a.id=j.payee_id
   JOIN core_payments.legacy_bases b ON b.id=NEW.basis_id
   WHERE j.id=NEW.journal_id AND j.kind='credit'
     AND j.scope='legacy_payments' AND j.operation_key=NEW.operation_id
     AND j.amount_units=b.credit_units AND a.owner_account_id=b.account_id
     AND a.bucket='wallet' AND (NEW.outcome->>'journal_id')::bigint=j.id
 ) THEN
   RAISE EXCEPTION 'legacy credit does not match frozen basis' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER legacy_application_receipt BEFORE INSERT ON core_payments.legacy_applications
 FOR EACH ROW EXECUTE FUNCTION core_payments.guard_legacy_application();

CREATE TRIGGER legacy_bases_immutable BEFORE UPDATE OR DELETE ON core_payments.legacy_bases
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_proofs_immutable BEFORE UPDATE OR DELETE ON core_payments.legacy_proofs
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER provider_identifiers_immutable BEFORE UPDATE OR DELETE ON core_payments.provider_identifiers
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_applications_immutable BEFORE UPDATE OR DELETE ON core_payments.legacy_applications
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_bases_no_truncate BEFORE TRUNCATE ON core_payments.legacy_bases
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_proofs_no_truncate BEFORE TRUNCATE ON core_payments.legacy_proofs
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER provider_identifiers_no_truncate BEFORE TRUNCATE ON core_payments.provider_identifiers
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_applications_no_truncate BEFORE TRUNCATE ON core_payments.legacy_applications
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_payments.legacy_bases,core_payments.legacy_proofs,
 core_payments.provider_identifiers,core_payments.legacy_applications FROM PUBLIC;
