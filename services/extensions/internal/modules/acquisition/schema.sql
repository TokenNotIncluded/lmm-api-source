-- Fresh isolated acquisition database only; installed offline with owner authority.
-- No Core or other-extension joins or foreign keys. User IDs are Core user IDs.
-- Analytics records only: no balance, reward, payment or credential state.
CREATE SCHEMA lmm_acquisition;
CREATE TABLE lmm_acquisition.contract (singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton), version integer NOT NULL);
INSERT INTO lmm_acquisition.contract VALUES (true, 1);

CREATE TABLE lmm_acquisition.config (
 singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
 started_at bigint NOT NULL CHECK (started_at>0),
 lookback_days integer NOT NULL CHECK (lookback_days BETWEEN 1 AND 90),
 last_cleanup_at bigint NOT NULL DEFAULT 0,
 payment_snapshot_updated_at bigint NOT NULL DEFAULT 0,
 payment_snapshot_status text NOT NULL DEFAULT '' CHECK (char_length(payment_snapshot_status)<=32)
);
-- Fresh installs start observing at installation; an import replaces this row.
INSERT INTO lmm_acquisition.config(started_at,lookback_days) VALUES (extract(epoch FROM statement_timestamp())::bigint,30);

CREATE TABLE lmm_acquisition.attribution_policies (
 id bigserial PRIMARY KEY,
 effective_at bigint NOT NULL CHECK (effective_at>0),
 lookback_days integer NOT NULL CHECK (lookback_days BETWEEN 1 AND 90)
);
CREATE INDEX ON lmm_acquisition.attribution_policies(effective_at);

-- Registration cohort snapshot: registration time and whether the account was
-- staff (administrator/root) or deleted when the snapshot was taken. Core has
-- no registration-time read for extensions; this is analytics metadata, never
-- authority. Current platform role always comes from Core per request.
CREATE TABLE lmm_acquisition.cohort (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 registered_at bigint NOT NULL CHECK (registered_at>0),
 staff boolean NOT NULL DEFAULT false,
 deleted boolean NOT NULL DEFAULT false,
 origin text NOT NULL CHECK (origin IN ('legacy_import'))
);
CREATE INDEX ON lmm_acquisition.cohort(registered_at);

CREATE TABLE lmm_acquisition.links (
 id text PRIMARY KEY CHECK (id ~ '^[0-9a-f]{32}$'),
 name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
 source text NOT NULL DEFAULT '' CHECK (char_length(source)<=80),
 medium text NOT NULL DEFAULT '' CHECK (char_length(medium)<=80),
 campaign text NOT NULL DEFAULT '' CHECK (char_length(campaign)<=80),
 content text NOT NULL DEFAULT '' CHECK (char_length(content)<=80),
 target text NOT NULL DEFAULT '' CHECK (char_length(target)<=80),
 archived boolean NOT NULL DEFAULT false,
 created_at bigint NOT NULL DEFAULT 0,
 deleted_at bigint NOT NULL DEFAULT 0
);
CREATE INDEX ON lmm_acquisition.links(created_at);

-- Browser identifiers are SHA-256 hashes of a random cookie, never raw cookies.
CREATE TABLE lmm_acquisition.visitors (
 id text PRIMARY KEY CHECK (id ~ '^[0-9a-f]{64}$'),
 user_id bigint NOT NULL DEFAULT 0 CHECK (user_id>=0),
 created_at bigint NOT NULL
);
CREATE INDEX ON lmm_acquisition.visitors(user_id);
CREATE INDEX ON lmm_acquisition.visitors(created_at);

CREATE TABLE lmm_acquisition.visits (
 id bigserial PRIMARY KEY,
 visitor_id text NOT NULL REFERENCES lmm_acquisition.visitors(id),
 nonce text NOT NULL CHECK (char_length(nonce)<=64),
 consent_version integer NOT NULL DEFAULT 1,
 link_id text NOT NULL DEFAULT '' CHECK (char_length(link_id)<=32),
 source text NOT NULL DEFAULT '' CHECK (char_length(source)<=253),
 medium text NOT NULL DEFAULT '' CHECK (char_length(medium)<=80),
 campaign text NOT NULL DEFAULT '' CHECK (char_length(campaign)<=80),
 content text NOT NULL DEFAULT '' CHECK (char_length(content)<=80),
 referrer_host text NOT NULL DEFAULT '' CHECK (char_length(referrer_host)<=253),
 landing text NOT NULL DEFAULT '' CHECK (char_length(landing)<=80),
 evidence text NOT NULL DEFAULT '' CHECK (char_length(evidence)<=32),
 created_at bigint NOT NULL,
 UNIQUE(visitor_id,nonce)
);
CREATE INDEX ON lmm_acquisition.visits(created_at);
CREATE INDEX ON lmm_acquisition.visits(source);

CREATE TABLE lmm_acquisition.accounts (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 consent_version integer NOT NULL DEFAULT 0,
 first_source text NOT NULL DEFAULT '' CHECK (char_length(first_source)<=253),
 first_evidence text NOT NULL DEFAULT '' CHECK (char_length(first_evidence)<=32),
 first_observed_at bigint NOT NULL DEFAULT 0,
 registration_inferred boolean NOT NULL DEFAULT false,
 first_visit_id bigint NOT NULL DEFAULT 0,
 registration_visit_id bigint NOT NULL DEFAULT 0,
 registration_source text NOT NULL DEFAULT 'unknown' CHECK (char_length(registration_source)<=253),
 registration_link_id text NOT NULL DEFAULT '' CHECK (char_length(registration_link_id)<=32),
 registration_content text NOT NULL DEFAULT '' CHECK (char_length(registration_content)<=80),
 registration_campaign text NOT NULL DEFAULT '' CHECK (char_length(registration_campaign)<=80),
 registration_evidence text NOT NULL DEFAULT '' CHECK (char_length(registration_evidence)<=32),
 registration_at bigint NOT NULL DEFAULT 0,
 attribution_rule text NOT NULL DEFAULT '' CHECK (char_length(attribution_rule)<=40),
 lookback_days integer NOT NULL DEFAULT 0,
 self_reported text NOT NULL DEFAULT '' CHECK (char_length(self_reported)<=80),
 self_reported_at bigint NOT NULL DEFAULT 0,
 created_at bigint NOT NULL DEFAULT 0
);
CREATE INDEX ON lmm_acquisition.accounts(registration_source);
CREATE INDEX ON lmm_acquisition.accounts(registration_link_id);

CREATE TABLE lmm_acquisition.consents (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 allowed boolean NOT NULL,
 version integer NOT NULL,
 updated_at bigint NOT NULL
);

CREATE TABLE lmm_acquisition.self_reports (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 source text NOT NULL CHECK (char_length(source) BETWEEN 1 AND 32),
 detail text NOT NULL DEFAULT '' CHECK (char_length(detail)<=160),
 updated_at bigint NOT NULL
);

CREATE TABLE lmm_acquisition.correction_heads (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 revision bigint NOT NULL DEFAULT 0,
 source text NOT NULL DEFAULT '' CHECK (char_length(source)<=80),
 updated_at bigint NOT NULL DEFAULT 0
);
-- Append-only audit (no runtime UPDATE grant); deleted only on withdrawal/retention.
CREATE TABLE lmm_acquisition.corrections (
 id bigserial PRIMARY KEY,
 user_id bigint NOT NULL CHECK (user_id>0),
 previous_revision bigint NOT NULL,
 previous_source text NOT NULL CHECK (char_length(previous_source)<=253),
 source text NOT NULL CHECK (char_length(source)<=80),
 reason text NOT NULL CHECK (char_length(reason)<=300),
 actor_id bigint NOT NULL CHECK (actor_id>0),
 created_at bigint NOT NULL
);
CREATE INDEX ON lmm_acquisition.corrections(user_id);

-- Operator-entered marketing spend for comparison; not a ledger or payment.
CREATE TABLE lmm_acquisition.costs (
 id bigserial PRIMARY KEY,
 link_id text NOT NULL REFERENCES lmm_acquisition.links(id),
 from_at bigint NOT NULL,
 to_at bigint NOT NULL,
 observation_days integer NOT NULL CHECK (observation_days BETWEEN 1 AND 90),
 currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
 amount_micros bigint NOT NULL CHECK (amount_micros BETWEEN 0 AND 1000000000000000),
 updated_by bigint NOT NULL CHECK (updated_by>0),
 updated_at bigint NOT NULL,
 UNIQUE(link_id,from_at,to_at,observation_days,currency)
);

-- Imported attribution snapshots of a first cash payment. Attribution only:
-- no amount, reward or balance. New snapshots need a Core payment read.
CREATE TABLE lmm_acquisition.first_payments (
 user_id bigint PRIMARY KEY CHECK (user_id>0),
 first_paid_at bigint NOT NULL DEFAULT 0,
 source text NOT NULL DEFAULT '',
 evidence text NOT NULL DEFAULT '',
 link_id text NOT NULL DEFAULT '',
 campaign text NOT NULL DEFAULT '',
 medium text NOT NULL DEFAULT '',
 content text NOT NULL DEFAULT '',
 referrer_host text NOT NULL DEFAULT '',
 visit_id bigint NOT NULL DEFAULT 0,
 observed_at bigint NOT NULL DEFAULT 0,
 inferred boolean NOT NULL DEFAULT false,
 lookback_days integer NOT NULL DEFAULT 0,
 policy_effective_at bigint NOT NULL DEFAULT 0,
 rule text NOT NULL DEFAULT '',
 history_conflict_at bigint NOT NULL DEFAULT 0,
 conflicting_paid_at bigint NOT NULL DEFAULT 0,
 snapshotted_at bigint NOT NULL DEFAULT 0
);

-- Imported daily first/last successful API response times (no request data).
CREATE TABLE lmm_acquisition.activities (
 user_id bigint NOT NULL CHECK (user_id>0),
 day bigint NOT NULL,
 first_at bigint NOT NULL,
 last_at bigint NOT NULL,
 PRIMARY KEY(user_id,day)
);
CREATE TABLE lmm_acquisition.activity_state (
 singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
 started_at bigint NOT NULL,
 scanned_through bigint NOT NULL,
 updated_at bigint NOT NULL,
 status text NOT NULL CHECK (char_length(status)<=32),
 incomplete boolean NOT NULL DEFAULT false,
 gap_from bigint NOT NULL DEFAULT 0,
 gap_through bigint NOT NULL DEFAULT 0
);
CREATE TABLE lmm_acquisition.activity_gaps (
 from_at bigint PRIMARY KEY,
 through_at bigint NOT NULL
);
