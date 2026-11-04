-- Offline owner-only migration; grant runtime SELECT, INSERT and UPDATE on the
-- new table separately after validating the target database and role.
BEGIN;
LOCK TABLE lmm_community.contract IN EXCLUSIVE MODE;
DO $$ BEGIN
 IF (SELECT version FROM lmm_community.contract WHERE singleton) IS DISTINCT FROM 1 THEN
  RAISE EXCEPTION 'community version 1 required';
 END IF;
END $$;
CREATE TABLE lmm_community.directory_ad_intents (
 request_id text PRIMARY KEY,
 owner_user_id bigint NOT NULL CHECK (owner_user_id>0),
 name text NOT NULL, url text NOT NULL, summary text NOT NULL, description text NOT NULL,
 bid_cents bigint NOT NULL CHECK (bid_cents BETWEEN 100 AND 1000000),
 content_sha256 text NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
 quote_id text NOT NULL DEFAULT '', quota bigint NOT NULL DEFAULT 0,
 quote_expires_at bigint NOT NULL DEFAULT 0,
 payment_id text NOT NULL DEFAULT '', command_sha256 text NOT NULL DEFAULT '',
 payment_state text NOT NULL DEFAULT 'preparing' CHECK (payment_state IN ('preparing','prepared','paid','rejected')),
 rejection_code text NOT NULL DEFAULT '',
 refund_request_id text NOT NULL DEFAULT '', refund_state text NOT NULL DEFAULT 'none' CHECK (refund_state IN ('none','pending','refunded')),
 refund_command_sha256 text NOT NULL DEFAULT ''
);
REVOKE ALL ON lmm_community.directory_ad_intents FROM PUBLIC;
UPDATE lmm_community.contract SET version=2 WHERE singleton;
COMMIT;
