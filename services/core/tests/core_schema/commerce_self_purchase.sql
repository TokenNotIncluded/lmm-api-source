-- Developer SQL evidence for the actual installed ledger, when a Rust test
-- binary is unavailable. Run only after fresh installation in an owned *_test
-- database. This does not establish HTTP/RPC or Go delivery acceptance.
BEGIN;
DO $$
DECLARE account BIGINT; wallet BIGINT; held BIGINT; reserve BIGINT; capture BIGINT; topup BIGINT; debt BIGINT;
    paid JSONB; settled JSONB; refunded JSONB; request JSONB; before_state JSONB;
    result JSONB; rejected BOOLEAN;
BEGIN
    IF right(current_database(),5)<>'_test' THEN
        RAISE EXCEPTION 'isolated test database required';
    END IF;
    INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id INTO account;
    result := core_billing.open_ledger_wallet(account);
    wallet := (result->>'wallet_id')::BIGINT;
    held := (result->>'reserved_id')::BIGINT;
    result := core_billing.post_ledger(jsonb_build_object('scope','self-commerce-test','operation_key','opening',
        'reason','Synthetic funds in an isolated test database',
        'action',jsonb_build_object('kind','credit','account_id',account,'amount_units',499999,'payment_reference','synthetic-self-purchase-opening')));
    topup := (result->>'journal_id')::BIGINT;
    result := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','insufficient',
        'reason','Self-purchase still requires the entire price',
        'action',jsonb_build_object('kind','reserve','account_id',account,'amount_units',500000)));
    IF result <> '{"status":"rejected","code":"insufficient_funds"}'::JSONB
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=wallet)<>499999
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=held)<>0 THEN
        RAISE EXCEPTION 'insufficient full price changed money';
    END IF;
    result := core_billing.post_ledger(jsonb_build_object('scope','self-commerce-test','operation_key','fund',
        'reason','Synthetic funds in an isolated test database',
        'action',jsonb_build_object('kind','credit','account_id',account,'amount_units',500001,'payment_reference','synthetic-self-purchase-fund')));
    paid := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','pay',
        'reason','Full self-purchase reservation',
        'action',jsonb_build_object('kind','reserve','account_id',account,'amount_units',500000)));
    reserve := (paid->>'journal_id')::BIGINT;
    IF paid->>'status'<>'posted'
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=wallet)<>500000
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=held)<>500000 THEN
        RAISE EXCEPTION 'full reserve was not funded';
    END IF;
    request := jsonb_build_object('scope','core-commerce','operation_key','settle',
        'reason','Delivered self-purchase at the current zero merchant fee',
        'action',jsonb_build_object('kind','commerce_capture','reservation_journal_id',reserve,'destination_account_id',account,'amount_units',500000));
    settled := core_billing.post_ledger(request);
    capture := (settled->>'journal_id')::BIGINT;
    IF settled->>'status'<>'posted' OR jsonb_array_length(settled->'entries')<>2
        OR core_billing.post_ledger(request)<>settled
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=wallet)<>1000000
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=held)<>0 THEN
        RAISE EXCEPTION 'self settlement did not return exactly the full hold once';
    END IF;
    SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) INTO before_state FROM core_billing.balance_state b;
    request := jsonb_build_object('scope','core-commerce','operation_key','refund',
        'reason','Refund exact original zero-fee same-wallet sale',
        'action',jsonb_build_object('kind','refund_commerce','journal_id',capture));
    refunded := core_billing.post_ledger(request);
    IF refunded->>'status'<>'posted' OR refunded->'entries'<>'[]'::JSONB
        OR core_billing.post_ledger(request)<>refunded
        OR (SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b)<>before_state
        OR NOT EXISTS(SELECT FROM core_billing.ledger_journals WHERE id=(refunded->>'journal_id')::BIGINT
            AND parent_journal_id=capture AND amount_units=500000 AND payer_id=wallet AND payee_id=wallet) THEN
        RAISE EXCEPTION 'self refund changed money or lacked immutable original evidence';
    END IF;
    result := core_billing.post_ledger(jsonb_set(request,'{operation_key}','"second-refund"'));
    IF result<>'{"status":"rejected","code":"already_compensated"}'::JSONB THEN
        RAISE EXCEPTION 'different key refunded the same sale twice';
    END IF;
    rejected := false;
    BEGIN
        PERFORM core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','self-transfer',
            'reason','Generic self-transfer remains forbidden',
            'action',jsonb_build_object('kind','transfer','from_account_id',account,'to_account_id',account,'amount_units',1)));
    EXCEPTION WHEN SQLSTATE 'P1000' THEN rejected := true; END;
    IF NOT rejected THEN RAISE EXCEPTION 'generic self-transfer was enabled'; END IF;
    paid := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','next-pay',
        'reason','Separate funded order',
        'action',jsonb_build_object('kind','reserve','account_id',account,'amount_units',500000)));
    reserve := (paid->>'journal_id')::BIGINT;
    request := jsonb_build_object('scope','unrelated-service','operation_key','wrong-scope',
        'reason','Other scopes cannot use same-wallet commerce',
        'action',jsonb_build_object('kind','commerce_capture','reservation_journal_id',reserve,'destination_account_id',account,'amount_units',500000));
    rejected := false;
    BEGIN PERFORM core_billing.post_ledger(request);
    EXCEPTION WHEN SQLSTATE 'P1000' THEN rejected := true; END;
    IF NOT rejected THEN RAISE EXCEPTION 'other scope acquired same-wallet settlement'; END IF;
    request := jsonb_set(jsonb_set(request,'{scope}','"core-commerce"'),'{action,amount_units}','1');
    rejected := false;
    BEGIN PERFORM core_billing.post_ledger(request);
    EXCEPTION WHEN SQLSTATE 'P1000' THEN rejected := true; END;
    IF NOT rejected THEN RAISE EXCEPTION 'partial self capture bypassed full original price'; END IF;
    result := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','cancel',
        'reason','Return only original held money after failed captures',
        'action',jsonb_build_object('kind','release','reservation_journal_id',reserve)));
    IF result->>'status'<>'posted'
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=wallet)<>1000000
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=held)<>0 THEN
        RAISE EXCEPTION 'rejected self capture consumed its reserve';
    END IF;
    -- A provider refund can create debt while the original purchase is held.
    -- The self-capture follows the existing incoming-wallet debt allocation;
    -- the later self-refund must not recreate debt or credit the wallet again.
    paid := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','debt-pay',
        'reason','Reserve the full wallet before a verified original topup refund',
        'action',jsonb_build_object('kind','reserve','account_id',account,'amount_units',1000000)));
    reserve := (paid->>'journal_id')::BIGINT;
    result := core_billing.post_ledger(jsonb_build_object('scope','self-commerce-test','operation_key','original-topup-refund',
        'reason','Synthetic provider refund tied to the original topup',
        'action',jsonb_build_object('kind','refund_topup','journal_id',topup,'amount_units',200000,'refund_reference','synthetic-self-purchase-debt')));
    SELECT id INTO debt FROM core_billing.ledger_accounts WHERE owner_account_id=account AND bucket='debt';
    IF result->>'status'<>'posted'
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=debt)<>-200000 THEN
        RAISE EXCEPTION 'original topup refund did not establish the debt fixture';
    END IF;
    settled := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','debt-settle',
        'reason','Self-capture repays only current wallet debt',
        'action',jsonb_build_object('kind','commerce_capture','reservation_journal_id',reserve,'destination_account_id',account,'amount_units',1000000)));
    capture := (settled->>'journal_id')::BIGINT;
    IF settled->>'status'<>'posted' OR jsonb_array_length(settled->'entries')<>3
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=wallet)<>800000
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=held)<>0
        OR (SELECT balance_units FROM core_billing.balance_state WHERE ledger_account_id=debt)<>0 THEN
        RAISE EXCEPTION 'self-capture did not conserve the held amount and debt repayment';
    END IF;
    SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) INTO before_state FROM core_billing.balance_state b;
    refunded := core_billing.post_ledger(jsonb_build_object('scope','core-commerce','operation_key','debt-refund',
        'reason','Same-wallet refund cannot undo prior debt repayment',
        'action',jsonb_build_object('kind','refund_commerce','journal_id',capture)));
    IF refunded->>'status'<>'posted' OR refunded->'entries'<>'[]'::JSONB
        OR (SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b)<>before_state THEN
        RAISE EXCEPTION 'self-refund recreated debt or changed wallet balance/revision';
    END IF;
END $$;
-- Force deferred balance and original-journal checks before reporting success.
SET CONSTRAINTS ALL IMMEDIATE;
COMMIT;
SELECT 'PASS: full-price reserve, same-wallet capture, exact no-credit refund, replay, scope restrictions and original-refund debt repayment';
