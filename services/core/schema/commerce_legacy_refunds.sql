-- Historical balance goods only. No escrow opening, native pay or wallet credit.
-- The offline owner reviews the actual source order/sale/refund set against the
-- same archive and identity mapping used by BOTH parties' opening receipts.
CREATE TABLE core_commerce.legacy_refund_grants (
    shop_id TEXT NOT NULL CHECK(shop_id ~ '^[0-9a-f]{64}$'),
    order_id TEXT PRIMARY KEY CHECK(order_id ~ '^[0-9a-f]{64}$'),
    migration_id TEXT NOT NULL REFERENCES core_billing.opening_import_runs(migration_id),
    buyer_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    seller_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    buyer_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    seller_account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
    quantity BIGINT NOT NULL CHECK(quantity>0),
    credits BIGINT NOT NULL CHECK(credits BETWEEN 1 AND 9007199254740991),
    source_row_sha256 TEXT NOT NULL CHECK(source_row_sha256 ~ '^[0-9a-f]{64}$'),
    request JSONB NOT NULL
);
CREATE TABLE core_commerce.legacy_refund_operations (
    order_id TEXT PRIMARY KEY REFERENCES core_commerce.legacy_refund_grants(order_id),
    operation_key TEXT NOT NULL UNIQUE CHECK(operation_key ~ '^[0-9a-f]{64}$'),
    command_sha256 BYTEA NOT NULL CHECK(octet_length(command_sha256)=32),
    command JSONB NOT NULL,
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    receipt JSONB NOT NULL
);
CREATE TRIGGER legacy_refund_grants_immutable BEFORE UPDATE OR DELETE ON core_commerce.legacy_refund_grants FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_refund_grants_no_truncate BEFORE TRUNCATE ON core_commerce.legacy_refund_grants FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_refund_operations_immutable BEFORE UPDATE OR DELETE ON core_commerce.legacy_refund_operations FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_refund_operations_no_truncate BEFORE TRUNCATE ON core_commerce.legacy_refund_operations FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();

CREATE FUNCTION core_commerce.import_legacy_refund_grant(p JSONB) RETURNS JSONB
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o JSONB:=p->'source_order'; sale JSONB:=p->'source_sale'; r JSONB;
    buyer BIGINT; seller BIGINT; buyer_user BIGINT; seller_user BIGINT; amount BIGINT; qty BIGINT; unit_price BIGINT;
    buyer_account BIGINT; seller_account BIGINT; prior core_commerce.legacy_refund_grants%ROWTYPE;
BEGIN
    IF current_user<>pg_get_userbyid((SELECT nspowner FROM pg_namespace WHERE nspname='core_commerce')) THEN
        RAISE EXCEPTION 'legacy store import requires offline owner' USING ERRCODE='42501';
    END IF;
    IF current_setting('transaction_isolation')<>'serializable' OR current_setting('synchronous_commit')<>'on' THEN
        RAISE EXCEPTION 'legacy store import requires durable serializable transaction' USING ERRCODE='25001';
    END IF;
    IF NOT COALESCE(octet_length(p::TEXT)<=65536 AND core_billing.legacy_escrow_keys(p,ARRAY[
        'migration_id','source_archive_sha256','mapping_sha256','observed_at','review_proof_sha256',
        'shop_id','source_row_sha256','source_sale_sha256','source_order','source_sale','source_refunds','buyer_user_id','seller_user_id'])
        AND core_billing.legacy_escrow_hash(p->'shop_id') AND core_billing.legacy_escrow_hash(p->'source_row_sha256')
        AND core_billing.legacy_escrow_hash(p->'source_sale_sha256') AND core_billing.legacy_escrow_hash(p->'review_proof_sha256')
        AND core_billing.legacy_escrow_keys(o,ARRAY['id','buyer_id','seller_id','quantity','unit_price_quota','price_quota','payment_method','status','paid_at','amount_minor','fee_quota','fee_held'])
        AND core_billing.legacy_escrow_hash(o->'id') AND o->>'status'='paid' AND o->>'payment_method'='balance'
        AND o->'amount_minor'='0'::JSONB AND o->'fee_held'='false'::JSONB
        AND core_billing.legacy_escrow_keys(sale,ARRAY['id','order_id','kind','from_user_id','to_user_id','quota'])
        AND jsonb_typeof(sale->'id')='string' AND length(sale->>'id') BETWEEN 1 AND 100
        AND sale->>'kind'='sale' AND sale->>'order_id'=o->>'id' AND jsonb_typeof(p->'source_refunds')='array',FALSE) THEN
        RAISE EXCEPTION 'requires actual unrefunded balance order and original sale' USING ERRCODE='P1000';
    END IF;
    buyer:=core_billing.legacy_escrow_id(o->'buyer_id'); seller:=core_billing.legacy_escrow_id(o->'seller_id');
    buyer_user:=core_billing.legacy_escrow_id(p->'buyer_user_id'); seller_user:=core_billing.legacy_escrow_id(p->'seller_user_id');
    qty:=core_billing.legacy_escrow_uint(o->'quantity',TRUE); amount:=core_billing.legacy_escrow_uint(o->'price_quota',TRUE);
    unit_price:=core_billing.legacy_escrow_uint(o->'unit_price_quota',TRUE);
    IF unit_price::NUMERIC*qty<>amount OR core_billing.legacy_escrow_uint(o->'paid_at',TRUE)>core_billing.legacy_escrow_uint(p->'observed_at',TRUE)
        OR core_billing.legacy_escrow_uint(o->'fee_quota')>amount
        OR core_billing.legacy_escrow_id(sale->'from_user_id')<>buyer OR core_billing.legacy_escrow_id(sale->'to_user_id')<>seller
        OR core_billing.legacy_escrow_uint(sale->'quota',TRUE)<>amount THEN
        RAISE EXCEPTION 'original paid principal or parties do not match' USING ERRCODE='P1001';
    END IF;
    -- ALL source refunds are reviewed. A pending reservation is not free cap;
    -- completed/UNKNOWN/inflight records cannot be hidden by a zero aggregate.
    FOR r IN SELECT value FROM jsonb_array_elements(p->'source_refunds') LOOP
        IF NOT COALESCE(r->>'order_id'=o->>'id' AND r->>'status' IN ('rejected','cancelled'),FALSE) THEN
            RAISE EXCEPTION 'source refund already consumes or reserves principal' USING ERRCODE='P1001';
        END IF;
    END LOOP;
    PERFORM pg_advisory_xact_lock(hashtextextended('lmm-opening-import-v1',0));
    IF NOT EXISTS(SELECT FROM core_billing.opening_import_runs b WHERE b.migration_id=p->>'migration_id'
        AND b.source_archive_sha256=p->>'source_archive_sha256' AND b.mapping_sha256=p->>'mapping_sha256'
        AND b.observed_at=(p->>'observed_at')::BIGINT) THEN
        RAISE EXCEPTION 'same reviewed opening run required' USING ERRCODE='P1001';
    END IF;
    IF NOT EXISTS(SELECT FROM core_migration.import_runs r WHERE r.migration_id=p->>'migration_id'
        AND r.source_archive_sha256=p->>'source_archive_sha256' AND r.mapping_sha256=p->>'mapping_sha256'
        AND r.manifest->>'scope' IN ('financial-opening-only-v1','financial-opening-with-legacy-escrow-v1')) THEN
        RAISE EXCEPTION 'completed financial migration receipt required' USING ERRCODE='P1001';
    END IF;
    SELECT personal_account_id INTO buyer_account FROM core_identity.users WHERE id=buyer_user;
    SELECT personal_account_id INTO seller_account FROM core_identity.users WHERE id=seller_user;
    IF buyer_account IS NULL OR seller_account IS NULL OR EXISTS(
        SELECT account FROM (VALUES(buyer_account),(seller_account)) parties(account)
        WHERE NOT EXISTS(SELECT FROM core_billing.opening_imports i WHERE i.migration_id=p->>'migration_id' AND i.account_id=parties.account)
    ) THEN RAISE EXCEPTION 'both original identities require matching opening receipts' USING ERRCODE='P1001'; END IF;
    IF NOT EXISTS(SELECT FROM core_billing.opening_imports i WHERE i.migration_id=p->>'migration_id'
        AND i.account_id=buyer_account AND i.source_entity_key IN ('user:'||buyer::TEXT,'legacy.users/'||buyer::TEXT))
        OR NOT EXISTS(SELECT FROM core_billing.opening_imports i WHERE i.migration_id=p->>'migration_id'
        AND i.account_id=seller_account AND i.source_entity_key IN ('user:'||seller::TEXT,'legacy.users/'||seller::TEXT)) THEN
        RAISE EXCEPTION 'opening receipt must bind original source user' USING ERRCODE='P1001';
    END IF;
    SELECT * INTO prior FROM core_commerce.legacy_refund_grants WHERE order_id=o->>'id';
    IF FOUND THEN
        IF prior.request IS DISTINCT FROM p THEN RAISE EXCEPTION 'historical authorization changed' USING ERRCODE='P1001'; END IF;
    ELSE
        INSERT INTO core_commerce.legacy_refund_grants VALUES(p->>'shop_id',o->>'id',p->>'migration_id',buyer_user,seller_user,
            buyer_account,seller_account,qty,amount,p->>'source_row_sha256',p);
    END IF;
    RETURN jsonb_build_object('status','imported','order_id',o->>'id');
END $$;

CREATE FUNCTION core_commerce.finalize_legacy_refund(p JSONB,hash BYTEA,actor BIGINT,account BIGINT) RETURNS JSONB
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE g core_commerce.legacy_refund_grants%ROWTYPE; prior core_commerce.legacy_refund_operations%ROWTYPE;
    outcome JSONB; receipt JSONB;
BEGIN
    IF current_setting('transaction_isolation')<>'serializable' OR current_setting('synchronous_commit')<>'on' THEN
        RAISE EXCEPTION 'legacy refund requires durable serializable transaction' USING ERRCODE='25001';
    END IF;
    IF NOT COALESCE(core_billing.legacy_escrow_keys(p,ARRAY['key','kind','shop_id','order_id','source_row_sha256','buyer','seller','credits','quantity'])
        AND p->>'kind'='legacy_refund' AND core_billing.legacy_escrow_hash(p->'key')
        AND core_billing.legacy_escrow_hash(p->'shop_id') AND core_billing.legacy_escrow_hash(p->'order_id')
        AND core_billing.legacy_escrow_hash(p->'source_row_sha256') AND octet_length(hash)=32,FALSE) THEN
        RAISE EXCEPTION 'invalid historical full refund' USING ERRCODE='P1000';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('legacy-store-refund:'||(p->>'order_id'),0));
    SELECT * INTO g FROM core_commerce.legacy_refund_grants WHERE order_id=p->>'order_id';
    IF NOT FOUND THEN RAISE EXCEPTION 'no historical store authorization' USING ERRCODE='P1002'; END IF;
    IF (g.seller_user_id,g.seller_account_id) IS DISTINCT FROM (actor,account) THEN
        RAISE EXCEPTION 'original seller Session required' USING ERRCODE='42501';
    END IF;
    IF g.shop_id<>p->>'shop_id' OR g.source_row_sha256<>p->>'source_row_sha256'
        OR p->'buyer' IS DISTINCT FROM jsonb_build_object('kind','personal','id',(g.request->'source_order'->>'buyer_id')::BIGINT)
        OR p->'seller' IS DISTINCT FROM jsonb_build_object('kind','personal','id',(g.request->'source_order'->>'seller_id')::BIGINT)
        OR core_billing.legacy_escrow_uint(p->'credits',TRUE)<>g.credits
        OR core_billing.legacy_escrow_uint(p->'quantity',TRUE)<>g.quantity THEN
        RAISE EXCEPTION 'frozen original order mismatch' USING ERRCODE='P1001';
    END IF;
    SELECT * INTO prior FROM core_commerce.legacy_refund_operations WHERE order_id=g.order_id;
    IF FOUND THEN
        IF prior.operation_key<>p->>'key' OR prior.command_sha256<>hash OR prior.command<>p OR prior.actor_user_id<>actor THEN
            RAISE EXCEPTION 'original refund command changed' USING ERRCODE='P1001';
        END IF;
        RETURN prior.receipt;
    END IF;
    -- Old balance refunds retain the original platform fee. Self-sale refunds
    -- have zero wallet delta, as in go-v0.2.118 merchant_store_refunds.go.
    IF g.buyer_account_id<>g.seller_account_id THEN
        outcome:=core_billing.post_ledger(jsonb_build_object('scope','core-legacy-store-refund','operation_key',p->>'key',
            'actor_user_id',actor,'reason','Historical store full refund '||g.order_id,
            'action',jsonb_build_object('kind','transfer','from_account_id',g.seller_account_id,'to_account_id',g.buyer_account_id,'amount_units',g.credits)));
        IF outcome->>'status' IS DISTINCT FROM 'posted' THEN
            -- Throwing rolls back the rejected ledger operation too. Retry only
            -- the same saved key after liquidity recovers; no false settlement.
            RAISE EXCEPTION 'original seller funds unavailable' USING ERRCODE='P1003';
        END IF;
    END IF;
    receipt:=jsonb_build_object('key',p->>'key','command_hash',encode(hash,'hex'),
        'reference','legacy-store-refund:'||(p->>'key'),'outcome','succeeded','rejection_code','');
    INSERT INTO core_commerce.legacy_refund_operations VALUES(g.order_id,p->>'key',hash,p,actor,receipt);
    RETURN receipt;
END $$;
REVOKE ALL ON core_commerce.legacy_refund_grants,core_commerce.legacy_refund_operations FROM PUBLIC;
REVOKE ALL ON FUNCTION core_commerce.import_legacy_refund_grant(JSONB),core_commerce.finalize_legacy_refund(JSONB,BYTEA,BIGINT,BIGINT) FROM PUBLIC;
