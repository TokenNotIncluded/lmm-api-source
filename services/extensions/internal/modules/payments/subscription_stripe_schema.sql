-- Explicit owner installation / offline upgrade only. Never startup DDL.
CREATE TABLE payment_stripe_subscription_checkouts (
 order_id TEXT PRIMARY KEY CHECK(order_id ~ '^sub_[a-f0-9]{64}$'),
 channel_id BIGINT NOT NULL CHECK(channel_id>0),
 frozen_sha256 TEXT NOT NULL CHECK(length(frozen_sha256)=64),
 frozen_order BYTEA NOT NULL CHECK(octet_length(frozen_order) BETWEEN 1 AND 65536),
 first_dispatch TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 checkout_id TEXT,
 checkout_url TEXT,
 checkout_expires BIGINT,
 recovered_at TIMESTAMPTZ,
 lookup_attempts INTEGER NOT NULL DEFAULT 0,
 next_lookup TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK((checkout_id IS NULL)=(checkout_url IS NULL))
);
CREATE TABLE payment_stripe_subscription_inbox (
 evidence_id TEXT PRIMARY KEY,
 order_id TEXT NOT NULL CHECK(order_id ~ '^sub_[a-f0-9]{64}$'),
 channel_id BIGINT NOT NULL CHECK(channel_id>0),
 event_id TEXT NOT NULL,
 payload BYTEA NOT NULL CHECK(octet_length(payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','accepted','rejected')),
 result BYTEA,
 attempts INTEGER NOT NULL DEFAULT 0,
 next_attempt TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT NOT NULL DEFAULT '',
 CHECK((state='accepted')=(result IS NOT NULL))
);
CREATE INDEX payment_stripe_subscription_checkout_due ON payment_stripe_subscription_checkouts(next_lookup,order_id) WHERE checkout_id IS NULL;
CREATE INDEX payment_stripe_subscription_due ON payment_stripe_subscription_inbox(next_attempt,evidence_id) WHERE state='pending';
-- Different legitimate delivery signatures may recover the same Core receipt.
-- Exactly-once invoice/grant identity is enforced in Core, not by raw arrival.
CREATE INDEX payment_stripe_subscription_verified_event ON payment_stripe_subscription_inbox(order_id,event_id) WHERE state='accepted';
CREATE FUNCTION guard_stripe_subscription_recovery() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'retained Stripe original recovery' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='payment_stripe_subscription_inbox' THEN
  IF (to_jsonb(NEW)-ARRAY['state','result','attempts','next_attempt','error_code']) IS DISTINCT FROM
   (to_jsonb(OLD)-ARRAY['state','result','attempts','next_attempt','error_code'])
   OR (OLD.state='accepted' AND (NEW.state,NEW.result) IS DISTINCT FROM (OLD.state,OLD.result)) THEN
   RAISE EXCEPTION 'immutable Stripe original evidence' USING ERRCODE='23514'; END IF;
 ELSE
  IF (to_jsonb(NEW)-ARRAY['checkout_id','checkout_url','checkout_expires','recovered_at','lookup_attempts','next_lookup']) IS DISTINCT FROM
   (to_jsonb(OLD)-ARRAY['checkout_id','checkout_url','checkout_expires','recovered_at','lookup_attempts','next_lookup'])
   OR (OLD.checkout_id IS NOT NULL AND (NEW.checkout_id,NEW.checkout_url,NEW.checkout_expires) IS DISTINCT FROM (OLD.checkout_id,OLD.checkout_url,OLD.checkout_expires)) THEN
   RAISE EXCEPTION 'immutable Stripe original checkout' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER stripe_subscription_checkout_immutable BEFORE UPDATE OR DELETE ON payment_stripe_subscription_checkouts FOR EACH ROW EXECUTE FUNCTION guard_stripe_subscription_recovery();
CREATE TRIGGER stripe_subscription_inbox_immutable BEFORE UPDATE OR DELETE ON payment_stripe_subscription_inbox FOR EACH ROW EXECUTE FUNCTION guard_stripe_subscription_recovery();
REVOKE ALL ON payment_stripe_subscription_checkouts,payment_stripe_subscription_inbox FROM PUBLIC;
REVOKE ALL ON FUNCTION guard_stripe_subscription_recovery() FROM PUBLIC;
