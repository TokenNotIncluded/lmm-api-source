-- Platform top-up coupons, independent from promotional gifts and store sales.
CREATE TABLE core_payments.discount_codes (
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 code TEXT NOT NULL UNIQUE CHECK(code ~ '^[A-Z0-9][A-Z0-9_-]{2,63}$'),
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 120),
 discount_percent INTEGER NOT NULL CHECK(discount_percent BETWEEN 1 AND 99),
 min_credit_units BIGINT NOT NULL CHECK(min_credit_units>=0),
 max_uses BIGINT NOT NULL CHECK(max_uses>=0),
 enabled BOOLEAN NOT NULL,
 starts_at BIGINT NOT NULL CHECK(starts_at>=0),
 ends_at BIGINT NOT NULL CHECK(ends_at=0 OR ends_at>starts_at),
 created_by BIGINT NOT NULL REFERENCES core_identity.users(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_payments.discount_reservations (
 intent_id BIGINT PRIMARY KEY REFERENCES core_payments.intents(id),
 code_id BIGINT NOT NULL REFERENCES core_payments.discount_codes(id),
 consumed BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX discount_reservations_code ON core_payments.discount_reservations(code_id);
CREATE FUNCTION core_payments.guard_discount_reservation() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' OR (NEW.intent_id,NEW.code_id) IS DISTINCT FROM (OLD.intent_id,OLD.code_id)
    OR OLD.consumed OR NOT NEW.consumed
    OR NOT EXISTS(SELECT FROM core_payments.intents WHERE id=NEW.intent_id AND credit_journal_id IS NOT NULL) THEN
  RAISE EXCEPTION 'immutable payment discount reservation' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER discount_reservation_identity BEFORE UPDATE OR DELETE ON core_payments.discount_reservations
 FOR EACH ROW EXECUTE FUNCTION core_payments.guard_discount_reservation();
CREATE FUNCTION core_payments.consume_discount() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.credit_journal_id IS NULL AND NEW.credit_journal_id IS NOT NULL THEN
  UPDATE core_payments.discount_reservations SET consumed=TRUE WHERE intent_id=NEW.id AND NOT consumed;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER payment_discount_consumption AFTER UPDATE ON core_payments.intents
 FOR EACH ROW EXECUTE FUNCTION core_payments.consume_discount();
REVOKE ALL ON core_payments.discount_codes,core_payments.discount_reservations FROM PUBLIC;
REVOKE ALL ON FUNCTION core_payments.guard_discount_reservation(),core_payments.consume_discount() FROM PUBLIC;

CREATE TABLE core_meta.payment_discount_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER payment_discount_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.payment_discount_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER payment_discount_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.payment_discount_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.payment_discount_upgrades FROM PUBLIC;
