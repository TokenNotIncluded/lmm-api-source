-- Explicit payment-store extension only; never installed by runtime.
-- Raw candidates do not own (order,event). Only a Core-confirmed grant can.
CREATE TABLE payment_subscription_inbox (
 evidence_id TEXT PRIMARY KEY CHECK(evidence_id ~ '^pay_[a-f0-9]{64}$'),
 order_id TEXT NOT NULL CHECK(order_id ~ '^sub_[a-f0-9]{64}$'),
 event_id TEXT NOT NULL,
 event_digest TEXT NOT NULL,
 payload BYTEA NOT NULL CHECK(octet_length(payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','granted','rejected')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
 next_attempt TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT NOT NULL DEFAULT '',
 proof_id TEXT,
 subscription_id BIGINT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK((state='granted')=(subscription_id IS NOT NULL)),
 CHECK(subscription_id IS NULL OR proof_id IS NOT NULL)
);
CREATE UNIQUE INDEX payment_subscription_granted_event ON payment_subscription_inbox(order_id,event_id) WHERE state='granted';
CREATE INDEX payment_subscription_work_due ON payment_subscription_inbox(next_attempt,evidence_id) WHERE state='pending';
REVOKE ALL ON payment_subscription_inbox FROM PUBLIC;
CREATE FUNCTION guard_payment_subscription_evidence() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' OR
  (to_jsonb(NEW)-ARRAY['state','attempts','next_attempt','error_code','proof_id','subscription_id']) IS DISTINCT FROM
  (to_jsonb(OLD)-ARRAY['state','attempts','next_attempt','error_code','proof_id','subscription_id']) OR
  (OLD.state='granted' AND (NEW.state,NEW.proof_id,NEW.subscription_id) IS DISTINCT FROM (OLD.state,OLD.proof_id,OLD.subscription_id)) THEN
  RAISE EXCEPTION 'immutable original subscription evidence and grant receipt' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER payment_subscription_evidence_immutable BEFORE UPDATE OR DELETE ON payment_subscription_inbox
 FOR EACH ROW EXECUTE FUNCTION guard_payment_subscription_evidence();
REVOKE ALL ON FUNCTION guard_payment_subscription_evidence() FROM PUBLIC;
