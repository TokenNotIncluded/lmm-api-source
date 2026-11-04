-- Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
-- Fresh installation only. Runtime may supply preview inputs and seal one result.
CREATE TABLE core_wallet.affiliate_claims (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    owner_user BIGINT NOT NULL REFERENCES core_identity.users(id),
    account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    session_id TEXT NOT NULL,
    amount BIGINT NOT NULL CHECK (amount >= 500000 AND amount <= 9007199254740991),
    csrf_digest BYTEA NOT NULL CHECK (octet_length(csrf_digest) = 32),
    expires_at BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    outcome JSONB
);
REVOKE ALL ON core_wallet.affiliate_claims FROM PUBLIC;
REVOKE ALL ON SEQUENCE core_wallet.affiliate_claims_id_seq FROM PUBLIC;

-- Owner-only ledger visibility; no caller-selected SQL or privileged mutation.
CREATE FUNCTION core_wallet.check_affiliate_claim() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE operation core_billing.ledger_operations%ROWTYPE;
BEGIN
    IF TG_OP IN ('DELETE', 'TRUNCATE') THEN
        RAISE EXCEPTION 'affiliate claim history is immutable' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.outcome IS NOT NULL THEN
            RAISE EXCEPTION 'affiliate claim must start without outcome' USING ERRCODE = '23514';
        END IF;
    ELSE
        IF (NEW.id, NEW.owner_user, NEW.account_id, NEW.session_id, NEW.amount,
            NEW.csrf_digest, NEW.expires_at, NEW.created_at)
            IS DISTINCT FROM
           (OLD.id, OLD.owner_user, OLD.account_id, OLD.session_id, OLD.amount,
            OLD.csrf_digest, OLD.expires_at, OLD.created_at)
           OR OLD.outcome IS NOT NULL OR NEW.outcome IS NULL THEN
            RAISE EXCEPTION 'affiliate claim permits one outcome only' USING ERRCODE = '23514';
        END IF;
    END IF;
    IF NOT EXISTS (SELECT FROM core_identity.users
        WHERE id = NEW.owner_user AND personal_account_id = NEW.account_id) THEN
        RAISE EXCEPTION 'affiliate claim personal account mismatch' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'INSERT' THEN RETURN NEW; END IF;

    SELECT * INTO operation FROM core_billing.ledger_operations
        WHERE scope = 'core-affiliate-claim' AND operation_key = 'claim-' || NEW.id::TEXT;
    IF NOT FOUND OR operation.result IS DISTINCT FROM NEW.outcome
        OR operation.request IS DISTINCT FROM jsonb_build_object(
            'scope', 'core-affiliate-claim', 'operation_key', 'claim-' || NEW.id::TEXT,
            'actor_user_id', NEW.owner_user, 'reason', 'Browser-confirmed affiliate claim',
            'action', jsonb_build_object('kind', 'claim_affiliate',
                'account_id', NEW.account_id, 'amount_units', NEW.amount)) THEN
        RAISE EXCEPTION 'affiliate claim ledger operation mismatch' USING ERRCODE = '23514';
    END IF;
    IF NEW.outcome->>'status' = 'rejected' THEN
        IF NEW.outcome IS DISTINCT FROM jsonb_build_object('status', 'rejected', 'code', NEW.outcome->>'code')
            OR NOT COALESCE(NEW.outcome->>'code' IN ('not_found', 'insufficient_funds',
                'invalid_original', 'exceeds_remaining', 'already_finalized',
                'already_compensated', 'amount_overflow', 'duplicate_payment', 'duplicate_refund'), FALSE) THEN
            RAISE EXCEPTION 'affiliate claim rejected outcome mismatch' USING ERRCODE = '23514';
        END IF;
    ELSIF NEW.outcome->>'status' = 'posted' THEN
        IF NOT EXISTS (
            SELECT FROM core_billing.ledger_journals j
            JOIN core_billing.ledger_accounts payer ON payer.id = j.payer_id
            JOIN core_billing.ledger_accounts payee ON payee.id = j.payee_id
            WHERE to_jsonb(j.id) = NEW.outcome->'journal_id'
              AND j.kind = 'claim_affiliate' AND j.scope = 'core-affiliate-claim'
              AND j.operation_key = 'claim-' || NEW.id::TEXT
              AND j.actor_user_id = NEW.owner_user AND j.amount_units = NEW.amount
              AND j.reason = 'Browser-confirmed affiliate claim'
              AND payer.owner_account_id = NEW.account_id AND payer.bucket = 'affiliate'
              AND payee.owner_account_id = NEW.account_id AND payee.bucket = 'wallet'
              -- Entries include the ledger's existing wallet/debt repayment split.
              AND NEW.outcome = (SELECT jsonb_build_object('status', 'posted', 'journal_id', j.id,
                  'entries', jsonb_agg(jsonb_build_object(
                      'ledger_account_id', e.ledger_account_id, 'delta_units', e.delta_units,
                      'balance_after_units', e.balance_after_units, 'balance_revision', e.balance_revision)
                      ORDER BY e.ledger_account_id))
                  FROM core_billing.ledger_entries e WHERE e.journal_id = j.id)
        ) THEN
            RAISE EXCEPTION 'affiliate claim posted outcome mismatch' USING ERRCODE = '23514';
        END IF;
    ELSE
        RAISE EXCEPTION 'affiliate claim outcome status mismatch' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION core_wallet.check_affiliate_claim() FROM PUBLIC;
CREATE TRIGGER affiliate_claim_guard BEFORE INSERT OR UPDATE OR DELETE ON core_wallet.affiliate_claims
    FOR EACH ROW EXECUTE FUNCTION core_wallet.check_affiliate_claim();
CREATE TRIGGER affiliate_claim_no_truncate BEFORE TRUNCATE ON core_wallet.affiliate_claims
    FOR EACH STATEMENT EXECUTE FUNCTION core_wallet.check_affiliate_claim();
