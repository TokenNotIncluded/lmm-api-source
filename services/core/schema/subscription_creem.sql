-- Additive Creem entitlement component; never wallet credit or Stripe events.
ALTER TABLE core_subscriptions.external_orders DROP CONSTRAINT external_orders_provider_check;
ALTER TABLE core_subscriptions.external_orders ADD CONSTRAINT external_orders_provider_check CHECK(provider IN ('epay','stripe','waffo_pancake','creem'));
ALTER TABLE core_subscriptions.external_prices DROP CONSTRAINT external_prices_currency_check;
ALTER TABLE core_subscriptions.external_prices ADD CONSTRAINT external_prices_currency_check CHECK(currency IN ('CNY','LDC','USD','EUR'));
ALTER TABLE core_subscriptions.external_orders DROP CONSTRAINT external_orders_currency_check;
ALTER TABLE core_subscriptions.external_orders ADD CONSTRAINT external_orders_currency_check CHECK(currency IN ('CNY','LDC','USD','EUR'));
CREATE TABLE core_subscriptions.creem_prices (
 plan_id BIGINT NOT NULL, plan_revision BIGINT NOT NULL, channel_id BIGINT NOT NULL,
 provider_price_id TEXT NOT NULL CHECK(provider_price_id ~ '^prod_[A-Za-z0-9_]+$'),
 interval TEXT NOT NULL CHECK(interval IN ('day','week','month','year')),
 interval_count BIGINT NOT NULL CHECK(interval_count BETWEEN 1 AND 365),
 success_url TEXT NOT NULL, cancel_url TEXT NOT NULL,
 PRIMARY KEY(plan_id,plan_revision,channel_id),
 FOREIGN KEY(plan_id,plan_revision,channel_id) REFERENCES core_subscriptions.external_prices(plan_id,plan_revision,channel_id)
);
CREATE TABLE core_subscriptions.creem_state (
 order_id TEXT PRIMARY KEY REFERENCES core_subscriptions.external_orders(id),
 provider_subscription_id TEXT UNIQUE,
 provider_customer_id TEXT,
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','trialing','active','past_due','unpaid','paused','canceling','canceled','expired','terminated')),
 event_created BIGINT NOT NULL DEFAULT 0,
 event_id TEXT NOT NULL DEFAULT '',
 period_start BIGINT NOT NULL DEFAULT 0,
 period_end BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE core_subscriptions.creem_events (
 order_id TEXT NOT NULL REFERENCES core_subscriptions.external_orders(id),
 event_id TEXT NOT NULL CHECK(event_id ~ '^evt_[A-Za-z0-9_]+$'),
 kind TEXT NOT NULL CHECK(kind IN ('paid','checkout','expired','failed','state')), event_created BIGINT NOT NULL CHECK(event_created>0),
 payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
 signed_payload BYTEA NOT NULL CHECK(octet_length(signed_payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
 CHECK(result ?& ARRAY['order_id','event_id','kind','state','transaction_id','subscription_id','period_start','period_end']),
 CHECK(payload_sha256=sha256(signed_payload)),
 CHECK(result->>'order_id'=order_id AND result->>'event_id'=event_id AND result->>'kind'=kind),
 PRIMARY KEY(order_id,event_id)
);
CREATE TABLE core_subscriptions.creem_cycles (
 order_id TEXT NOT NULL REFERENCES core_subscriptions.external_orders(id),
 transaction_id TEXT NOT NULL CHECK(transaction_id ~ '^tran_[A-Za-z0-9_]+$'),
 provider_subscription_id TEXT NOT NULL CHECK(provider_subscription_id ~ '^sub_[A-Za-z0-9_]+$'),
 period_start BIGINT NOT NULL CHECK(period_start>0), period_end BIGINT NOT NULL CHECK(period_end>period_start AND period_end<=253402300799),
 subscription_id BIGINT NOT NULL UNIQUE REFERENCES core_charging.subscriptions(id),
 event_id TEXT NOT NULL,
 PRIMARY KEY(order_id,transaction_id), UNIQUE(order_id,period_start,period_end),
 FOREIGN KEY(order_id,event_id) REFERENCES core_subscriptions.creem_events(order_id,event_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE FUNCTION core_subscriptions.check_creem_cycle() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o core_subscriptions.external_orders%ROWTYPE; s core_charging.subscriptions%ROWTYPE; terms JSONB;
BEGIN
 SELECT * INTO STRICT o FROM core_subscriptions.external_orders WHERE id=NEW.order_id;
 SELECT * INTO STRICT s FROM core_charging.subscriptions WHERE id=NEW.subscription_id;
 SELECT r.terms INTO STRICT terms FROM core_subscriptions.plan_revisions r WHERE r.plan_id=o.plan_id AND r.revision=o.plan_revision;
 IF o.provider<>'creem' OR o.confirmed_at IS NULL OR s.account_id<>o.account_id
  OR s.grant_reference<>'creem-subscription:'||o.id||':'||NEW.transaction_id
  OR s.anchor<>NEW.period_start OR s.ends_at<>NEW.period_end
  OR s.limit_credits<>(terms->>'limit_units')::bigint OR s.unlimited OR s.reset_basis<>'same'
  OR s.period<>terms->>'reset_period' OR s.seconds<>(terms->>'reset_seconds')::bigint
  OR to_jsonb(s.models)<>terms->'models' OR to_jsonb(s.groups)<>terms->'groups'
  OR NOT EXISTS(SELECT FROM core_subscriptions.creem_events e WHERE e.order_id=o.id AND e.event_id=NEW.event_id
   AND e.kind='paid' AND e.result->>'transaction_id'=NEW.transaction_id AND (e.result->>'subscription_id')::bigint=NEW.subscription_id
   AND (e.result->>'period_start')::bigint=NEW.period_start AND (e.result->>'period_end')::bigint=NEW.period_end)
  OR EXISTS(SELECT FROM core_subscriptions.creem_cycles prior WHERE prior.order_id=o.id AND prior.transaction_id<>NEW.transaction_id
   AND prior.period_start<NEW.period_end AND prior.period_end>NEW.period_start)
  OR NOT EXISTS(SELECT FROM core_payments.provider_identifiers i WHERE i.provider='creem' AND i.merchant=o.merchant
   AND i.environment=o.environment AND i.kind='payment' AND i.identifier=NEW.transaction_id AND i.subscription_order_id=o.id)
 THEN RAISE EXCEPTION 'Creem cycle requires original paid proof and frozen plan' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER creem_cycle_bound AFTER INSERT ON core_subscriptions.creem_cycles
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_subscriptions.check_creem_cycle();
CREATE FUNCTION core_subscriptions.guard_creem_state() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'retained Creem state' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT FROM core_subscriptions.external_orders WHERE id=NEW.order_id AND provider='creem' AND currency IN ('USD','CNY','EUR')) THEN
  RAISE EXCEPTION 'Creem state requires original Creem order' USING ERRCODE='23514'; END IF;
 IF TG_OP='UPDATE' AND (NEW.order_id<>OLD.order_id OR
  (OLD.provider_subscription_id IS NOT NULL AND NEW.provider_subscription_id IS DISTINCT FROM OLD.provider_subscription_id) OR
  (OLD.provider_customer_id IS NOT NULL AND NEW.provider_customer_id IS DISTINCT FROM OLD.provider_customer_id) OR
  NEW.event_created<OLD.event_created OR NEW.period_end<OLD.period_end) THEN
  RAISE EXCEPTION 'immutable Creem subscription binding and monotonic period' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER creem_state_guard BEFORE INSERT OR UPDATE OR DELETE ON core_subscriptions.creem_state FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_creem_state();
CREATE TRIGGER creem_prices_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.creem_prices FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER creem_events_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.creem_events FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER creem_cycles_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.creem_cycles FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER creem_prices_no_truncate BEFORE TRUNCATE ON core_subscriptions.creem_prices FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER creem_events_no_truncate BEFORE TRUNCATE ON core_subscriptions.creem_events FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER creem_cycles_no_truncate BEFORE TRUNCATE ON core_subscriptions.creem_cycles FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER creem_state_no_truncate BEFORE TRUNCATE ON core_subscriptions.creem_state FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_subscriptions.creem_prices,core_subscriptions.creem_state,core_subscriptions.creem_events,core_subscriptions.creem_cycles FROM PUBLIC;
REVOKE ALL ON FUNCTION core_subscriptions.check_creem_cycle(),core_subscriptions.guard_creem_state() FROM PUBLIC;

-- Owner-only offline 73 -> 74 receipt, never runtime upgrade authority.
CREATE TABLE core_meta.subscription_creem_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER subscription_creem_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.subscription_creem_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER subscription_creem_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.subscription_creem_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.subscription_creem_upgrades FROM PUBLIC;
