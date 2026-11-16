-- Explicit component 53 / affiliate ledger revision v1; first 52 components frozen.
ALTER TABLE core_billing.ledger_journals DROP CONSTRAINT ledger_journals_kind_check;
ALTER TABLE core_billing.ledger_journals ADD CONSTRAINT ledger_journals_kind_check CHECK(kind IN ('gift', 'credit', 'transfer', 'charge', 'reserve', 'capture', 'commerce_capture', 'split_commerce_capture', 'claim_affiliate', 'opening', 'release', 'refund', 'refund_topup', 'refund_commerce', 'reverse', 'legacy_escrow_opening', 'legacy_escrow_payout', 'legacy_escrow_release', 'affiliate_award','affiliate_clawback'));
CREATE OR REPLACE FUNCTION core_billing.ledger_apply_entry() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE old_units BIGINT; old_revision BIGINT; account_bucket TEXT; journal_kind TEXT;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM core_billing.ledger_journals j
        WHERE j.id = NEW.journal_id AND j.created_xid = pg_current_xact_id()
          AND NOT EXISTS (SELECT 1 FROM core_billing.ledger_operations o
              WHERE o.scope = j.scope AND o.operation_key = j.operation_key)
    ) THEN
        RAISE EXCEPTION 'journal is already sealed' USING ERRCODE = '55000';
    END IF;
    SELECT b.balance_units, b.revision, a.bucket INTO STRICT old_units, old_revision, account_bucket
        FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id = b.ledger_account_id
        WHERE b.ledger_account_id = NEW.ledger_account_id FOR NO KEY UPDATE OF b;
    NEW.balance_after_units := old_units + NEW.delta_units;
    NEW.balance_revision := old_revision + 1;
    SELECT kind INTO STRICT journal_kind FROM core_billing.ledger_journals WHERE id = NEW.journal_id;
    IF (account_bucket IN ('wallet', 'reserved', 'revenue') AND NEW.balance_after_units < 0)
        OR (account_bucket = 'debt' AND NEW.balance_after_units > 0)
        OR (account_bucket = 'debt' AND NEW.balance_after_units < old_units AND journal_kind NOT IN ('opening','refund_topup','refund_commerce'))
        OR (account_bucket = 'affiliate' AND NEW.balance_after_units < 0
            AND journal_kind NOT IN ('opening','affiliate_clawback') AND NEW.balance_after_units < old_units) THEN
        RAISE EXCEPTION 'insufficient funds' USING ERRCODE = 'P1003';
    END IF;
    UPDATE core_billing.balance_state SET balance_units = NEW.balance_after_units, revision = NEW.balance_revision
        WHERE ledger_account_id = NEW.ledger_account_id;
    RETURN NEW;
END $$;
