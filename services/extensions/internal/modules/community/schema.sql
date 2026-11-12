-- Fresh isolated community database only; installed offline with owner authority.
-- No Core or other-extension joins or foreign keys. User IDs are Core user IDs.
CREATE SCHEMA lmm_community;
CREATE TABLE lmm_community.contract (singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton), version integer NOT NULL);
INSERT INTO lmm_community.contract VALUES (true, 3);

CREATE TABLE lmm_community.directory_settings (
 singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
 links jsonb NOT NULL CHECK (jsonb_typeof(links)='array' AND jsonb_array_length(links)<=60),
 updated_by bigint NOT NULL CHECK (updated_by>0),
 updated_at bigint NOT NULL CHECK (updated_at>0)
);

-- Paid placements. Payment and refunds belong to Core; this table records the
-- placement and, when hidden without a Core refund, the refund still owed.
CREATE TABLE lmm_community.directory_ads (
 id bigserial PRIMARY KEY,
 owner_user_id bigint NOT NULL CHECK (owner_user_id>0),
 name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
 url text NOT NULL CHECK (octet_length(url) BETWEEN 1 AND 2048),
 summary text NOT NULL DEFAULT '' CHECK (char_length(summary)<=180),
 description text NOT NULL DEFAULT '' CHECK (char_length(description)<=1200),
 bid_cents bigint NOT NULL CHECK (bid_cents>0),
 charged_quota bigint NOT NULL CHECK (charged_quota>0),
 request_id text NOT NULL UNIQUE,
 status text NOT NULL CHECK (status IN ('active','hidden')),
 paid_at bigint NOT NULL CHECK (paid_at>0),
 expires_at bigint NOT NULL CHECK (expires_at>paid_at),
 hidden_at bigint NOT NULL DEFAULT 0,
 hidden_by bigint NOT NULL DEFAULT 0,
 refunded_at bigint NOT NULL DEFAULT 0,
 refund_status text NOT NULL DEFAULT 'none' CHECK (refund_status IN ('none','refunded','pending_core'))
);
CREATE INDEX directory_ads_active ON lmm_community.directory_ads (status, expires_at, charged_quota DESC, paid_at, id);
CREATE INDEX directory_ads_owner ON lmm_community.directory_ads (owner_user_id, paid_at DESC, id DESC);

-- Frozen request facts and Core receipts; never store user credentials.
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

CREATE TABLE lmm_community.profile_shares (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 token text NOT NULL UNIQUE CHECK (token ~ '^[0-9a-f]{48}$'),
 created_at bigint NOT NULL CHECK (created_at>0),
 model_usage_enabled boolean NOT NULL DEFAULT false,
 aggregate_usage_enabled boolean NOT NULL DEFAULT false,
 linked_profiles jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(linked_profiles)='array' AND jsonb_array_length(linked_profiles)<=5)
);

CREATE TABLE lmm_community.onboarding_todos (
 id bigserial PRIMARY KEY,
 user_id bigint NOT NULL UNIQUE CHECK (user_id>0),
 key_proven_at bigint NOT NULL DEFAULT 0,
 client_installed_at bigint NOT NULL DEFAULT 0,
 client_configured_at bigint NOT NULL DEFAULT 0,
 completed_at bigint NOT NULL DEFAULT 0,
 created_at bigint NOT NULL,
 updated_at bigint NOT NULL
);

CREATE TABLE lmm_community.signal_attempts (
 id bigserial PRIMARY KEY,
 token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
 seed bigint NOT NULL CHECK (seed BETWEEN 0 AND 4294967295),
 size integer NOT NULL,
 day text NOT NULL,
 actor text NOT NULL CHECK (actor IN ('human','ai')),
 model_id text NOT NULL DEFAULT '',
 harness text NOT NULL DEFAULT '',
 agent_name text NOT NULL DEFAULT '',
 started_at bigint NOT NULL,
 finished_at bigint NOT NULL DEFAULT 0,
 moves integer NOT NULL DEFAULT 0,
 elapsed_ms bigint NOT NULL DEFAULT 0,
 actions text NOT NULL DEFAULT '',
 created_at bigint NOT NULL
);
CREATE INDEX signal_attempts_unfinished ON lmm_community.signal_attempts (created_at) WHERE finished_at=0;

CREATE TABLE lmm_community.signal_records (
 id bigserial PRIMARY KEY,
 user_id bigint NOT NULL CHECK (user_id>0),
 attempt_id bigint UNIQUE REFERENCES lmm_community.signal_attempts(id),
 mode text NOT NULL CHECK (mode IN ('challenge','practice')),
 day text NOT NULL DEFAULT '',
 size integer NOT NULL,
 seed bigint NOT NULL,
 rules_version integer NOT NULL,
 moves integer NOT NULL,
 elapsed_ms bigint NOT NULL,
 hints integer NOT NULL DEFAULT 0,
 actor text NOT NULL,
 model_id text NOT NULL DEFAULT '',
 harness text NOT NULL DEFAULT '',
 agent_name text NOT NULL DEFAULT '',
 participant_key text NOT NULL,
 email text NOT NULL DEFAULT '',
 note text NOT NULL DEFAULT '' CHECK (char_length(note)<=160),
 actions text NOT NULL,
 digest text NOT NULL UNIQUE,
 public boolean NOT NULL,
 player_name text NOT NULL DEFAULT '' CHECK (char_length(player_name)<=80),
 created_at bigint NOT NULL
);
CREATE INDEX signal_records_user ON lmm_community.signal_records (user_id, created_at);
CREATE INDEX signal_records_daily ON lmm_community.signal_records (day, size, public);
REVOKE ALL ON SCHEMA lmm_community FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA lmm_community FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA lmm_community FROM PUBLIC;

-- Site presentation only; no balance, permission or credential authority.
CREATE TABLE lmm_community.navigation (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
 value text NOT NULL DEFAULT '{}' CHECK(octet_length(value)<=65536 AND jsonb_typeof(value::jsonb)='object'),
 revision bigint NOT NULL DEFAULT 1 CHECK(revision>0),
 updated_by bigint CHECK(updated_by>0),
 updated_at bigint NOT NULL DEFAULT 0 CHECK(updated_at>=0)
);
INSERT INTO lmm_community.navigation(singleton) VALUES(true);
REVOKE ALL ON lmm_community.navigation FROM PUBLIC;
