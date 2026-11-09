-- Synthetic PostgreSQL facts for the actual administration queries. No mock
-- success response, imported fiat valuation or production account is used.
BEGIN ISOLATION LEVEL SERIALIZABLE;
INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(8301,'personal'),(8302,'personal'),(8303,'personal');
INSERT INTO core_identity.users(id,personal_account_id,platform_role)
VALUES(8301,8301,'superadmin'),(8302,8302,'user'),(8303,8303,'user');
INSERT INTO core_identity.user_profiles(user_id,username,email)
VALUES(8301,'synthetic-admin',''),(8302,'synthetic-buyer','buyer@example.test'),(8303,'other-buyer','');
INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at)
OVERRIDING SYSTEM VALUE VALUES(9101,decode(repeat('51',32),'hex'),'api_key',8302,1,8302,clock_timestamp()+interval '1 hour');
DO $$
DECLARE at bigint := floor(extract(epoch FROM clock_timestamp()))::bigint;
    terms jsonb; charged jsonb; quote text := repeat('a',64);
BEGIN
    IF right(current_database(),5)<>'_test' THEN RAISE EXCEPTION 'isolated fixture required'; END IF;
    terms := '{"title":"Synthetic plan","price_units":"100000","duration_seconds":3600,"duration_months":0,
        "reset_period":"grant","reset_seconds":0,"limit_units":"1000","models":["*"],"groups":["*"],
        "max_purchases_per_user":0,"enabled":true}'::jsonb;
    INSERT INTO core_subscriptions.plans(id,revision) OVERRIDING SYSTEM VALUE VALUES(101,1);
    INSERT INTO core_subscriptions.plan_revisions VALUES(101,1,terms,8301,at);
    PERFORM core_billing.open_ledger_wallet(8302);
    PERFORM core_billing.post_ledger(jsonb_build_object('scope','synthetic-subscription-admin','operation_key','fund',
        'actor_user_id',8302,'reason','Synthetic test wallet','action',jsonb_build_object('kind','credit',
            'account_id',8302,'amount_units',500000,'payment_reference','synthetic-subscription-admin-funds')));
    charged := core_billing.post_ledger(jsonb_build_object('scope','core-subscription-wallet','operation_key','buy:'||quote,
        'actor_user_id',8302,'reason','Synthetic subscription sale','action',jsonb_build_object('kind','charge',
            'account_id',8302,'amount_units',100000)));
    IF charged->>'status'<>'posted' THEN RAISE EXCEPTION 'real ledger fixture charge failed'; END IF;
    INSERT INTO core_subscriptions.quotes(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,starts_at,ends_at,expires_at,state,result)
    VALUES(quote,8302,8302,'synthetic_purchase_original',101,1,at-100,at+3500,at+300,'posted','{}');
    INSERT INTO core_charging.subscriptions(id,account_id,grant_reference,period,anchor,ends_at,limit_credits,models,groups)
    OVERRIDING SYSTEM VALUE VALUES(501,8302,'subscription:'||quote,'grant',at-100,at+3500,1000,ARRAY['*'],ARRAY['*']);
    INSERT INTO core_subscriptions.purchases(quote_id,actor_user_id,account_id,subscription_id,charge_journal_id)
    VALUES(quote,8302,8302,501,(charged->>'journal_id')::bigint);
    INSERT INTO core_charging.subscriptions(id,account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,
        reset_basis,reset_limit_credits,models,groups)
    OVERRIDING SYSTEM VALUE VALUES(502,8302,'legacy-sub:synthetic-import:42','custom',at-60,3600,at+7200,700,'explicit',900,ARRAY['*'],ARRAY['*']);
    INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference)
    VALUES(502,at-60,at+60,40,'legacy-sub:synthetic-import:42');
    INSERT INTO core_charging.subscriptions(id,account_id,grant_reference,period,anchor,ends_at,limit_credits,models,groups)
    OVERRIDING SYSTEM VALUE VALUES
        (503,8302,'legacy-sub:synthetic-import:expired','grant',at-7200,at-3600,500,ARRAY['*'],ARRAY['*']),
        (504,8303,'synthetic-future-grant','grant',at+3600,at+7200,600,ARRAY['*'],ARRAY['*']),
        (505,8302,'synthetic-retained-grant','grant',at-60,at+3600,800,ARRAY['*'],ARRAY['*']);
    INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,
        fingerprint,worker_digest,spec_digest,price,source_subscription_id,source_start,source_end,
        authorized_at,lease_until,hard_deadline,state,reserved,settled,refunded,outcome)
    SELECT name,8302,9101,8302,8302,decode(repeat('11',32),'hex'),decode(repeat('22',32),'hex'),decode(repeat('33',32),'hex'),
        '{}',501,at-100,at+3500,at-30,at+300,at+1000,state,reserved,settled,refunded,outcome
    FROM (VALUES('synthetic-settled','settled',100,70,5,'completed'),
        ('synthetic-reserved','reserved',40,0,0,NULL),('synthetic-released','released',10,0,0,NULL))
        AS facts(name,state,reserved,settled,refunded,outcome);
END $$;
SET CONSTRAINTS ALL IMMEDIATE;
COMMIT;
