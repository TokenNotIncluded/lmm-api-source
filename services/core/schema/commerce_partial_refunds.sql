-- STORE02. Root registers this component AFTER commerce-self-purchase.
-- Keep historical ledger/commerce component bytes and successor targets intact.
CREATE TABLE core_commerce.refund_totals (
    order_id BIGINT PRIMARY KEY REFERENCES core_commerce.orders(id),
    refunded_units BIGINT NOT NULL DEFAULT 0 CHECK (refunded_units>=0),
    refunded_native BIGINT NOT NULL DEFAULT 0 CHECK (refunded_native>=0),
    retired_quantity BIGINT NOT NULL DEFAULT 0 CHECK (retired_quantity>=0),
    quantity_basis BIGINT NOT NULL DEFAULT 0 CHECK (quantity_basis>=0),
    quantity_native BIGINT NOT NULL DEFAULT 0 CHECK (quantity_native>=0),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision>=0)
);
CREATE TABLE core_commerce.refund_operations (
    operation_key TEXT PRIMARY KEY CHECK (operation_key ~ '^[a-f0-9]{64}$'),
    refund_id TEXT NOT NULL CHECK (refund_id ~ '^[a-f0-9]{64}$'),
    order_id BIGINT NOT NULL REFERENCES core_commerce.orders(id),
    command_sha256 BYTEA NOT NULL CHECK (octet_length(command_sha256)=32),
    command JSONB NOT NULL,
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    amount_units BIGINT NOT NULL CHECK (amount_units>0),
    amount_native BIGINT NOT NULL CHECK (amount_native>0),
    retired_quantity BIGINT NOT NULL CHECK (retired_quantity>=0),
    journal_id BIGINT REFERENCES core_billing.ledger_journals(id),
    receipt JSONB NOT NULL,
    UNIQUE(order_id,refund_id)
);
CREATE TRIGGER refund_operations_immutable BEFORE UPDATE OR DELETE ON core_commerce.refund_operations
    FOR EACH ROW EXECUTE FUNCTION core_commerce.immutable();
CREATE TRIGGER refund_operations_no_truncate BEFORE TRUNCATE ON core_commerce.refund_operations
    FOR EACH STATEMENT EXECUTE FUNCTION core_commerce.immutable();

-- This port has NO caller-selected payer, recipient or entry list. Rust checks
-- the current seller authority and exact original economics in the SAME tx.
-- The SQL independently locks the original order and its cumulative ceiling.
CREATE FUNCTION core_commerce.post_refund_delta(
    original_order BIGINT, p_operation_key TEXT, frozen_request JSONB,
    actor_id BIGINT, units BIGINT, capture_remaining BOOLEAN
) RETURNS BIGINT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
    o core_commerce.orders%ROWTYPE;
    reservation core_billing.ledger_journals%ROWTYPE;
    captured core_billing.ledger_journals%ROWTYPE;
    previous core_billing.ledger_operations%ROWTYPE;
    returned BIGINT; journal BIGINT; parent BIGINT; payer BIGINT; payee BIGINT;
    source_balance BIGINT; source_wallet BIGINT; source_debt BIGINT;
    recipient_debt BIGINT; recipient_debt_balance BIGINT;
    restored NUMERIC; original_repayment BIGINT; debit_wallet BIGINT; debit_debt BIGINT;
    incoming_debt BIGINT; incoming_wallet BIGINT;
    journal_kind TEXT; legs JSONB; leg RECORD;
BEGIN
    IF p_operation_key IS NULL OR p_operation_key !~ '^[a-f0-9]{64}$' OR actor_id IS NULL OR actor_id<=0 OR units IS NULL OR units<=0 OR frozen_request IS NULL OR capture_remaining IS NULL THEN
        RAISE EXCEPTION 'invalid original-order money operation' USING ERRCODE='P1000';
    END IF;
    SELECT * INTO STRICT o FROM core_commerce.orders WHERE id=original_order FOR UPDATE;
    SELECT * INTO STRICT reservation FROM core_billing.ledger_journals WHERE id=o.reserve_journal_id FOR NO KEY UPDATE;
    IF reservation.kind<>'reserve' OR reservation.scope<>'core-commerce' OR reservation.amount_units<>o.credits THEN
        RAISE EXCEPTION 'original reservation changed' USING ERRCODE='P1004';
    END IF;
    INSERT INTO core_commerce.refund_totals(order_id) VALUES(o.id) ON CONFLICT DO NOTHING;
    SELECT refunded_units INTO returned FROM core_commerce.refund_totals WHERE order_id=o.id FOR UPDATE;
    SELECT * INTO previous FROM core_billing.ledger_operations
        WHERE scope='core-commerce' AND ledger_operations.operation_key=p_operation_key;
    IF FOUND THEN
        IF previous.request IS DISTINCT FROM frozen_request THEN
            RAISE EXCEPTION 'original operation changed' USING ERRCODE='P1001';
        END IF;
        IF previous.result->>'status'<>'posted' THEN
            RAISE EXCEPTION 'original operation has no successful receipt' USING ERRCODE='P1004';
        END IF;
        RETURN (previous.result->>'journal_id')::BIGINT;
    END IF;
    IF units>o.credits-returned THEN
        RAISE EXCEPTION 'cumulative refund exceeds original principal' USING ERRCODE='P1005';
    END IF;
    -- Lock every affected wallet/debt/hold in the same ascending order as the
    -- existing ledger. Incoming money always repays its recipient's debt first.
    PERFORM b.ledger_account_id FROM core_billing.balance_state b
      JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id
      WHERE a.owner_account_id IN (o.buyer_account_id,o.seller_account_id)
      ORDER BY b.ledger_account_id FOR NO KEY UPDATE OF b;
    IF capture_remaining THEN
        IF o.state<>'paid' OR o.settlement_journal_id IS NOT NULL OR units<>o.credits-returned THEN
            RAISE EXCEPTION 'not the original remaining capture' USING ERRCODE='P1004';
        END IF;
        parent:=reservation.id; journal_kind:='commerce_capture';
        payer:=reservation.payer_id; source_wallet:=reservation.payee_id;
        SELECT id INTO STRICT payee FROM core_billing.ledger_accounts
            WHERE owner_account_id=o.seller_account_id AND bucket='wallet';
        debit_wallet:=units; debit_debt:=0;
    ELSIF o.state='paid' AND o.settlement_journal_id IS NULL THEN
        -- Before capture, return only this refund's slice of the original hold.
        parent:=reservation.id; journal_kind:='refund_commerce';
        payer:=reservation.payee_id; payee:=reservation.payer_id;
        source_wallet:=payer; debit_wallet:=units; debit_debt:=0;
    ELSIF o.state='settled' AND o.settlement_journal_id IS NOT NULL THEN
        SELECT * INTO STRICT captured FROM core_billing.ledger_journals WHERE id=o.settlement_journal_id FOR NO KEY UPDATE;
        IF captured.kind<>'commerce_capture' OR captured.scope<>'core-commerce'
          OR captured.parent_journal_id<>reservation.id OR captured.payer_id<>reservation.payer_id THEN
            RAISE EXCEPTION 'original capture changed' USING ERRCODE='P1004';
        END IF;
        -- A zero-fee same-wallet partial refund has no economic movement.
        -- Its immutable Core refund receipt/counters are the audit record;
        -- do not weaken the existing full-refund zero-entry journal gate.
        IF captured.payer_id=captured.payee_id THEN RETURN NULL; END IF;
        parent:=captured.id; journal_kind:='refund_commerce';
        payer:=captured.payee_id; payee:=captured.payer_id; source_wallet:=payer;
        SELECT a.id,b.balance_units INTO STRICT source_debt,source_balance FROM core_billing.ledger_accounts a
            JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
            WHERE a.owner_account_id=o.seller_account_id AND a.bucket='debt';
        SELECT least(balance_units,units) INTO STRICT debit_wallet FROM core_billing.balance_state WHERE ledger_account_id=source_wallet;
        debit_debt:=units-debit_wallet;
        SELECT coalesce(sum(e.delta_units),0) INTO original_repayment FROM core_billing.ledger_entries e
            WHERE e.journal_id=captured.id AND e.ledger_account_id=source_debt AND e.delta_units>0;
        SELECT coalesce(sum(-e.delta_units),0) INTO restored FROM core_billing.ledger_entries e
            JOIN core_billing.ledger_journals j ON j.id=e.journal_id
            WHERE j.parent_journal_id=captured.id AND j.kind IN ('refund','refund_commerce','reverse')
              AND e.ledger_account_id=source_debt AND e.delta_units<0;
        IF debit_debt>original_repayment-restored THEN
            -- Spent seller revenue cannot become arbitrary new debt. Rust
            -- rolls this tx back and leaves the exact refund UNKNOWN/pending.
            RAISE EXCEPTION 'original seller funds temporarily unavailable' USING ERRCODE='P1003';
        END IF;
    ELSE
        RAISE EXCEPTION 'original order cannot be refunded' USING ERRCODE='P1004';
    END IF;
    SELECT a.id,b.balance_units INTO STRICT recipient_debt,recipient_debt_balance
        FROM core_billing.ledger_accounts wallet JOIN core_billing.ledger_accounts a
          ON a.owner_account_id=wallet.owner_account_id AND a.bucket='debt'
        JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE wallet.id=payee;
    incoming_debt:=least(units,greatest(-recipient_debt_balance,0)); incoming_wallet:=units-incoming_debt;
    legs:=jsonb_build_array(jsonb_build_object('id',source_wallet,'units',-debit_wallet),
        jsonb_build_object('id',payee,'units',incoming_wallet),
        jsonb_build_object('id',recipient_debt,'units',incoming_debt));
    IF debit_debt>0 THEN legs:=legs||jsonb_build_array(jsonb_build_object('id',source_debt,'units',-debit_debt)); END IF;
    INSERT INTO core_billing.ledger_journals(scope,operation_key,kind,payer_id,payee_id,amount_units,parent_journal_id,actor_user_id,reason)
      VALUES('core-commerce',p_operation_key,journal_kind,payer,payee,units,parent,actor_id,'Original order partial refund/capture') RETURNING id INTO journal;
    FOR leg IN SELECT (value->>'id')::BIGINT AS id,sum((value->>'units')::BIGINT) AS delta
        FROM jsonb_array_elements(legs) GROUP BY (value->>'id')::BIGINT
        HAVING sum((value->>'units')::BIGINT)<>0 ORDER BY (value->>'id')::BIGINT LOOP
        INSERT INTO core_billing.ledger_entries(journal_id,ledger_account_id,delta_units) VALUES(journal,leg.id,leg.delta);
    END LOOP;
    INSERT INTO core_billing.ledger_operations(scope,operation_key,request,result)
        VALUES('core-commerce',p_operation_key,frozen_request,jsonb_build_object('status','posted','journal_id',journal,'entries','[]'::jsonb));
    RETURN journal;
END $$;
REVOKE ALL ON core_commerce.refund_totals,core_commerce.refund_operations FROM PUBLIC;
REVOKE ALL ON FUNCTION core_commerce.post_refund_delta(BIGINT,TEXT,JSONB,BIGINT,BIGINT,BOOLEAN) FROM PUBLIC;
