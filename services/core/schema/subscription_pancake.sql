-- Additive Pancake subscriptions. Version/count/pins are assigned by the root
-- integration owner. This component depends on subscription-epay, not Stripe.
ALTER TABLE core_subscriptions.external_prices DROP CONSTRAINT external_prices_currency_check;
ALTER TABLE core_subscriptions.external_prices ADD CONSTRAINT external_prices_currency_check CHECK(currency IN('CNY','LDC','USD'));
ALTER TABLE core_subscriptions.external_orders DROP CONSTRAINT external_orders_currency_check;
ALTER TABLE core_subscriptions.external_orders ADD CONSTRAINT external_orders_currency_check CHECK(currency IN('CNY','LDC','USD'));
ALTER TABLE core_subscriptions.external_orders DROP CONSTRAINT external_orders_provider_check;
ALTER TABLE core_subscriptions.external_orders ADD CONSTRAINT external_orders_provider_check CHECK(provider IN('epay','waffo_pancake','stripe'));

CREATE TABLE core_subscriptions.pancake_prices (
 plan_id BIGINT NOT NULL,plan_revision BIGINT NOT NULL,channel_id BIGINT NOT NULL,
 store_id TEXT NOT NULL CHECK(length(store_id) BETWEEN 1 AND 128),
 product_id TEXT NOT NULL CHECK(length(product_id) BETWEEN 1 AND 128),
 product_type TEXT NOT NULL CHECK(product_type IN('one_time','subscription')),
 billing_period TEXT NOT NULL CHECK(billing_period IN('','weekly','monthly','quarterly','yearly')),
 provider_plan_reference TEXT NOT NULL CHECK(length(provider_plan_reference) BETWEEN 1 AND 128),
 return_url TEXT NOT NULL CHECK(length(return_url) BETWEEN 1 AND 2048),
 checkout_region TEXT NOT NULL CHECK(checkout_region IN('global','china')),
 checkout_language TEXT NOT NULL CHECK(length(checkout_language) BETWEEN 1 AND 32),
 tax_category TEXT NOT NULL CHECK(tax_category IN('digital_goods','saas','software','ebook','online_course','consulting','professional_service')),
 PRIMARY KEY(plan_id,plan_revision,channel_id),
 FOREIGN KEY(plan_id,plan_revision,channel_id) REFERENCES core_subscriptions.external_prices(plan_id,plan_revision,channel_id)
);
CREATE TABLE core_subscriptions.pancake_orders (
 order_id TEXT PRIMARY KEY REFERENCES core_subscriptions.external_orders(id),
 channel_id BIGINT NOT NULL,
 provider_reference TEXT NOT NULL CHECK(provider_reference ~ '^[A-Za-z0-9_.-]{1,128}$'),
 binding JSONB NOT NULL CHECK(jsonb_typeof(binding)='object'),
 frozen_plan JSONB NOT NULL CHECK(jsonb_typeof(frozen_plan)='object' AND coalesce(jsonb_typeof(frozen_plan->'archived')='boolean',false)),
 -- Native originals have no archive receipt. An owner-reviewed legacy importer
 -- records the source archive/row digest and exact old order reference here.
 legacy_source JSONB CHECK(legacy_source IS NULL OR jsonb_typeof(legacy_source)='object'),
 provider_order_id TEXT,
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN('pending','active','canceling','past_due','canceled','historical_paid')),
 event_created BIGINT NOT NULL DEFAULT 0 CHECK(event_created>=0),
 event_id TEXT NOT NULL DEFAULT '',
 paid_period_start BIGINT NOT NULL DEFAULT 0,
 paid_period_end BIGINT NOT NULL DEFAULT 0 CHECK(paid_period_end>=paid_period_start),
 UNIQUE(provider_reference),UNIQUE(channel_id,provider_order_id)
);
CREATE TABLE core_subscriptions.pancake_events (
 order_id TEXT NOT NULL REFERENCES core_subscriptions.pancake_orders(order_id),
 event_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN('paid','historical_paid','checkout','state')),
 event_created BIGINT NOT NULL CHECK(event_created>=0),
 payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
 signed_payload BYTEA NOT NULL CHECK(octet_length(signed_payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
 PRIMARY KEY(order_id,event_id),
 CHECK(payload_sha256=sha256(signed_payload)),
 CHECK(result->>'order_id'=order_id AND result->>'event_id'=event_id)
);
CREATE TABLE core_subscriptions.pancake_cycles (
 order_id TEXT NOT NULL REFERENCES core_subscriptions.pancake_orders(order_id),
 cycle_key TEXT NOT NULL CHECK(length(cycle_key) BETWEEN 1 AND 160),
 payment_id TEXT CHECK(payment_id IS NULL OR length(payment_id) BETWEEN 1 AND 128),
 provider_order_id TEXT NOT NULL CHECK(length(provider_order_id) BETWEEN 1 AND 128),
 period_start BIGINT NOT NULL CHECK(period_start>0),
 period_end BIGINT NOT NULL CHECK(period_end>period_start),
 subscription_id BIGINT NOT NULL UNIQUE REFERENCES core_charging.subscriptions(id),
 event_id TEXT NOT NULL,
 PRIMARY KEY(order_id,cycle_key),UNIQUE(order_id,payment_id),UNIQUE(order_id,period_start,period_end),
 FOREIGN KEY(order_id,event_id) REFERENCES core_subscriptions.pancake_events(order_id,event_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE FUNCTION core_subscriptions.guard_pancake_order() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o core_subscriptions.external_orders%ROWTYPE; p core_subscriptions.pancake_prices%ROWTYPE;
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'retained Pancake original' USING ERRCODE='23514'; END IF;
 IF TG_OP='UPDATE' THEN
  IF (to_jsonb(NEW)-ARRAY['provider_order_id','status','event_created','event_id','paid_period_start','paid_period_end']) IS DISTINCT FROM
    (to_jsonb(OLD)-ARRAY['provider_order_id','status','event_created','event_id','paid_period_start','paid_period_end'])
    OR (OLD.provider_order_id IS NOT NULL AND NEW.provider_order_id IS DISTINCT FROM OLD.provider_order_id)
    OR NEW.event_created<OLD.event_created OR NEW.paid_period_end<OLD.paid_period_end
    OR (OLD.status='canceled' AND NEW.status<>'canceled')
  THEN RAISE EXCEPTION 'immutable Pancake binding or terminal state' USING ERRCODE='23514'; END IF;
 ELSE
  SELECT * INTO STRICT o FROM core_subscriptions.external_orders WHERE id=NEW.order_id;
  SELECT * INTO STRICT p FROM core_subscriptions.pancake_prices WHERE plan_id=o.plan_id AND plan_revision=o.plan_revision AND channel_id=o.channel_id;
  IF o.provider<>'waffo_pancake' OR o.channel_id<>NEW.channel_id
   OR NEW.frozen_plan->>'id' IS DISTINCT FROM o.plan_id::text
   OR NEW.frozen_plan->>'revision' IS DISTINCT FROM o.plan_revision::text
   OR NEW.frozen_plan->'terms' IS DISTINCT FROM (SELECT terms FROM core_subscriptions.plan_revisions WHERE plan_id=o.plan_id AND revision=o.plan_revision)
   OR NEW.binding->>'merchant' IS DISTINCT FROM o.merchant OR NEW.binding->>'environment' IS DISTINCT FROM o.environment
   OR NEW.binding->'price'->>'currency' IS DISTINCT FROM o.currency
   OR NEW.binding->'price'->>'amount_minor' IS DISTINCT FROM o.amount_minor::text
   OR NEW.binding->'price'->>'store_id' IS DISTINCT FROM p.store_id
   OR NEW.binding->'price'->>'product_id' IS DISTINCT FROM p.product_id
   OR NEW.binding->'price'->>'billing_period' IS DISTINCT FROM p.billing_period
   OR NEW.binding->'price'->>'product_type' IS DISTINCT FROM p.product_type
   OR NEW.binding->'price'->>'provider_plan_reference' IS DISTINCT FROM p.provider_plan_reference
  THEN RAISE EXCEPTION 'Pancake binding differs from original price' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER pancake_order_bound BEFORE INSERT OR UPDATE OR DELETE ON core_subscriptions.pancake_orders FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_pancake_order();
CREATE FUNCTION core_subscriptions.check_pancake_cycle() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o core_subscriptions.external_orders%ROWTYPE; b core_subscriptions.pancake_orders%ROWTYPE;
 s core_charging.subscriptions%ROWTYPE; terms JSONB; e core_subscriptions.pancake_events%ROWTYPE;
BEGIN
 SELECT * INTO STRICT o FROM core_subscriptions.external_orders WHERE id=NEW.order_id;
 SELECT * INTO STRICT b FROM core_subscriptions.pancake_orders WHERE order_id=NEW.order_id;
 SELECT * INTO STRICT s FROM core_charging.subscriptions WHERE id=NEW.subscription_id;
 SELECT * INTO STRICT e FROM core_subscriptions.pancake_events WHERE order_id=NEW.order_id AND event_id=NEW.event_id;
 SELECT r.terms INTO STRICT terms FROM core_subscriptions.plan_revisions r WHERE r.plan_id=o.plan_id AND r.revision=o.plan_revision;
 IF o.confirmed_at IS NULL OR o.provider<>'waffo_pancake' OR s.account_id<>o.account_id
  OR s.grant_reference<>'pancake-subscription:'||o.id||':'||NEW.cycle_key
  OR s.anchor<>NEW.period_start OR s.ends_at<>NEW.period_end
  OR s.period<>terms->>'reset_period' OR s.seconds<>(terms->>'reset_seconds')::bigint
  OR s.limit_credits<>(terms->>'limit_units')::bigint OR s.unlimited OR s.reset_basis<>'same'
  OR to_jsonb(s.models)<>terms->'models' OR to_jsonb(s.groups)<>terms->'groups'
  OR e.kind<>'paid' OR (e.result->>'subscription_id')::bigint IS DISTINCT FROM s.id
  OR e.result->>'cycle_key' IS DISTINCT FROM NEW.cycle_key
  OR EXISTS(SELECT FROM core_subscriptions.pancake_cycles c WHERE c.order_id=NEW.order_id AND c.cycle_key<>NEW.cycle_key
    AND (b.binding->'price'->>'product_type'='one_time' OR (c.period_start<NEW.period_end AND c.period_end>NEW.period_start)))
  OR (b.binding->'price'->>'product_type'='one_time' AND NEW.period_end<>core_subscriptions.external_ends_at(terms,NEW.period_start))
  OR NOT EXISTS(SELECT FROM core_payments.provider_identifiers i WHERE i.provider=o.provider AND i.merchant=o.merchant AND i.environment=o.environment AND i.kind='order' AND i.identifier=NEW.provider_order_id AND i.subscription_order_id=o.id)
  OR (NEW.payment_id IS NOT NULL AND NOT EXISTS(SELECT FROM core_payments.provider_identifiers i WHERE i.provider=o.provider AND i.merchant=o.merchant AND i.environment=o.environment AND i.kind='payment' AND i.identifier=NEW.payment_id AND i.subscription_order_id=o.id))
 THEN RAISE EXCEPTION 'Pancake cycle does not match original proof and plan' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER pancake_cycle_bound AFTER INSERT ON core_subscriptions.pancake_cycles DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_subscriptions.check_pancake_cycle();
CREATE TRIGGER pancake_prices_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.pancake_prices FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_events_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.pancake_events FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_cycles_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.pancake_cycles FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_prices_no_truncate BEFORE TRUNCATE ON core_subscriptions.pancake_prices FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_orders_no_truncate BEFORE TRUNCATE ON core_subscriptions.pancake_orders FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_events_no_truncate BEFORE TRUNCATE ON core_subscriptions.pancake_events FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER pancake_cycles_no_truncate BEFORE TRUNCATE ON core_subscriptions.pancake_cycles FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_subscriptions.pancake_prices,core_subscriptions.pancake_orders,core_subscriptions.pancake_events,core_subscriptions.pancake_cycles FROM PUBLIC;
REVOKE ALL ON FUNCTION core_subscriptions.guard_pancake_order(),core_subscriptions.check_pancake_cycle() FROM PUBLIC;
