-- Append-only successor component; Root assigns its global version/target.
-- Do not edit the installed Core59 EPay component or create another wallet.
ALTER TABLE core_subscriptions.external_prices DROP CONSTRAINT external_prices_currency_check;
ALTER TABLE core_subscriptions.external_prices ADD CONSTRAINT external_prices_currency_check
 CHECK(currency IN ('CNY','LDC','USD'));
ALTER TABLE core_subscriptions.external_orders DROP CONSTRAINT external_orders_provider_check;
ALTER TABLE core_subscriptions.external_orders ADD CONSTRAINT external_orders_provider_check CHECK(provider IN ('epay','stripe'));
ALTER TABLE core_subscriptions.external_orders DROP CONSTRAINT external_orders_currency_check;
ALTER TABLE core_subscriptions.external_orders ADD CONSTRAINT external_orders_currency_check CHECK(currency IN ('CNY','LDC','USD'));
CREATE TABLE core_subscriptions.stripe_prices (
 plan_id BIGINT NOT NULL, plan_revision BIGINT NOT NULL, channel_id BIGINT NOT NULL,
 provider_price_id TEXT NOT NULL CHECK(provider_price_id ~ '^price_[A-Za-z0-9_]+$'),
 interval TEXT NOT NULL CHECK(interval IN ('day','week','month','year')),
 interval_count BIGINT NOT NULL CHECK(interval_count BETWEEN 1 AND 365),
 success_url TEXT NOT NULL, cancel_url TEXT NOT NULL,
 PRIMARY KEY(plan_id,plan_revision,channel_id),
 FOREIGN KEY(plan_id,plan_revision,channel_id) REFERENCES core_subscriptions.external_prices(plan_id,plan_revision,channel_id)
);
CREATE TABLE core_subscriptions.stripe_state (
 order_id TEXT PRIMARY KEY REFERENCES core_subscriptions.external_orders(id),
 provider_subscription_id TEXT UNIQUE,
 provider_customer_id TEXT,
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','incomplete','incomplete_expired','trialing','active','past_due','unpaid','paused','canceling','canceled','expired')),
 event_created BIGINT NOT NULL DEFAULT 0,
 event_id TEXT NOT NULL DEFAULT '',
 period_start BIGINT NOT NULL DEFAULT 0,
 period_end BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE core_subscriptions.stripe_events (
 order_id TEXT NOT NULL REFERENCES core_subscriptions.external_orders(id),
 event_id TEXT NOT NULL CHECK(event_id ~ '^evt_[A-Za-z0-9_]+$'),
 kind TEXT NOT NULL CHECK(kind IN ('paid','checkout','expired','failed','state')), event_created BIGINT NOT NULL CHECK(event_created>0),
 payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
 signed_payload BYTEA NOT NULL CHECK(octet_length(signed_payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
 CHECK(result ?& ARRAY['order_id','event_id','kind','state','invoice_id','subscription_id','period_start','period_end']),
 CHECK(payload_sha256=sha256(signed_payload)),
 CHECK(result->>'order_id'=order_id AND result->>'event_id'=event_id AND result->>'kind'=kind),
 PRIMARY KEY(order_id,event_id)
);
CREATE TABLE core_subscriptions.stripe_cycles (
 order_id TEXT NOT NULL REFERENCES core_subscriptions.external_orders(id),
 invoice_id TEXT NOT NULL CHECK(invoice_id ~ '^in_[A-Za-z0-9_]+$'), payment_id TEXT NOT NULL CHECK(payment_id ~ '^pi_[A-Za-z0-9_]+$'),
 provider_subscription_id TEXT NOT NULL CHECK(provider_subscription_id ~ '^sub_[A-Za-z0-9_]+$'),
 period_start BIGINT NOT NULL CHECK(period_start>0), period_end BIGINT NOT NULL CHECK(period_end>period_start AND period_end<=253402300799),
 subscription_id BIGINT NOT NULL UNIQUE REFERENCES core_charging.subscriptions(id),
 event_id TEXT NOT NULL,
 PRIMARY KEY(order_id,invoice_id), UNIQUE(order_id,period_start,period_end),
 FOREIGN KEY(order_id,event_id) REFERENCES core_subscriptions.stripe_events(order_id,event_id) DEFERRABLE INITIALLY DEFERRED
);
-- Existing EPay function is provider-specific even after its shared order table
-- accepts Stripe. Reject accidental cross-provider direct SQL invocation.
CREATE FUNCTION core_subscriptions.guard_epay_provider() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NOT EXISTS(SELECT FROM core_subscriptions.external_orders WHERE id=NEW.order_id AND provider='epay') THEN
  RAISE EXCEPTION 'EPay proof requires an EPay order' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER external_proof_provider BEFORE INSERT ON core_subscriptions.external_proofs
 FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_epay_provider();
CREATE FUNCTION core_subscriptions.check_stripe_cycle() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o core_subscriptions.external_orders%ROWTYPE; s core_charging.subscriptions%ROWTYPE; terms JSONB;
BEGIN
 SELECT * INTO STRICT o FROM core_subscriptions.external_orders WHERE id=NEW.order_id;
 SELECT * INTO STRICT s FROM core_charging.subscriptions WHERE id=NEW.subscription_id;
 SELECT r.terms INTO STRICT terms FROM core_subscriptions.plan_revisions r WHERE r.plan_id=o.plan_id AND r.revision=o.plan_revision;
 IF o.provider<>'stripe' OR o.confirmed_at IS NULL OR s.account_id<>o.account_id
  OR s.grant_reference<>'stripe-subscription:'||o.id||':'||NEW.invoice_id
  OR s.anchor<>NEW.period_start OR s.ends_at<>NEW.period_end
  OR s.limit_credits<>(terms->>'limit_units')::bigint OR s.unlimited OR s.reset_basis<>'same'
  OR s.period<>terms->>'reset_period' OR s.seconds<>(terms->>'reset_seconds')::bigint
  OR to_jsonb(s.models)<>terms->'models' OR to_jsonb(s.groups)<>terms->'groups'
  OR NOT EXISTS(SELECT FROM core_subscriptions.stripe_events e WHERE e.order_id=o.id AND e.event_id=NEW.event_id
   AND e.kind='paid' AND e.result->>'invoice_id'=NEW.invoice_id AND (e.result->>'subscription_id')::bigint=NEW.subscription_id
   AND (e.result->>'period_start')::bigint=NEW.period_start AND (e.result->>'period_end')::bigint=NEW.period_end)
  OR EXISTS(SELECT FROM core_subscriptions.stripe_cycles prior WHERE prior.order_id=o.id AND prior.invoice_id<>NEW.invoice_id
   AND prior.period_start<NEW.period_end AND prior.period_end>NEW.period_start)
  OR NOT EXISTS(SELECT FROM core_payments.provider_identifiers i WHERE i.provider='stripe' AND i.merchant=o.merchant
   AND i.environment=o.environment AND i.kind='payment' AND i.identifier=NEW.payment_id AND i.subscription_order_id=o.id)
 THEN RAISE EXCEPTION 'Stripe cycle requires original paid proof and frozen plan' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER stripe_cycle_bound AFTER INSERT ON core_subscriptions.stripe_cycles
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_subscriptions.check_stripe_cycle();
CREATE FUNCTION core_subscriptions.guard_stripe_state() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'retained Stripe state' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT FROM core_subscriptions.external_orders WHERE id=NEW.order_id AND provider='stripe' AND currency='USD') THEN
  RAISE EXCEPTION 'Stripe state requires original Stripe order' USING ERRCODE='23514'; END IF;
 IF TG_OP='UPDATE' AND (NEW.order_id<>OLD.order_id OR
  (OLD.provider_subscription_id IS NOT NULL AND NEW.provider_subscription_id IS DISTINCT FROM OLD.provider_subscription_id) OR
  (OLD.provider_customer_id IS NOT NULL AND NEW.provider_customer_id IS DISTINCT FROM OLD.provider_customer_id) OR
  NEW.event_created<OLD.event_created OR NEW.period_end<OLD.period_end) THEN
  RAISE EXCEPTION 'immutable Stripe subscription binding and monotonic period' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER stripe_state_guard BEFORE INSERT OR UPDATE OR DELETE ON core_subscriptions.stripe_state FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_stripe_state();
CREATE TRIGGER stripe_prices_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.stripe_prices FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER stripe_events_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.stripe_events FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER stripe_cycles_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.stripe_cycles FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER stripe_prices_no_truncate BEFORE TRUNCATE ON core_subscriptions.stripe_prices FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER stripe_events_no_truncate BEFORE TRUNCATE ON core_subscriptions.stripe_events FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER stripe_cycles_no_truncate BEFORE TRUNCATE ON core_subscriptions.stripe_cycles FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER stripe_state_no_truncate BEFORE TRUNCATE ON core_subscriptions.stripe_state FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_subscriptions.stripe_prices,core_subscriptions.stripe_state,core_subscriptions.stripe_events,core_subscriptions.stripe_cycles FROM PUBLIC;
REVOKE ALL ON FUNCTION core_subscriptions.guard_epay_provider(),core_subscriptions.check_stripe_cycle(),core_subscriptions.guard_stripe_state() FROM PUBLIC;
