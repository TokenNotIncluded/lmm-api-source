-- Fresh installation only. Run explicitly using a module-only database role.
CREATE SCHEMA lmm_assistant;
CREATE TABLE lmm_assistant.contract (singleton boolean PRIMARY KEY CHECK (singleton), version integer NOT NULL CHECK (version = 3));
INSERT INTO lmm_assistant.contract VALUES (true, 3);
CREATE TABLE lmm_assistant.records (
    scope text NOT NULL CHECK (length(scope) BETWEEN 1 AND 256),
    bucket text NOT NULL CHECK (bucket IN ('sessions','calls','preferences','entries')),
    id text NOT NULL CHECK (length(id) BETWEEN 1 AND 256),
    body jsonb NOT NULL CHECK (jsonb_typeof(body) = 'object' AND octet_length(body::text) <= 16384),
    PRIMARY KEY (scope, bucket, id)
);
-- There are no user, credential, wallet, balance or ledger tables here.

-- Version 3 preserves legacy human authorship; existing v2 databases use the
-- explicit offline history upgrade, never startup DDL.
-- Numeric IDs stay within JavaScript's exact integer range for the legacy Web.
CREATE TABLE lmm_assistant.conversations (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    user_id bigint NOT NULL CHECK (user_id > 0),
    title text NOT NULL CHECK (char_length(title) BETWEEN 0 AND 255),
    last_message_preview text NOT NULL CHECK (char_length(last_message_preview) BETWEEN 0 AND 512),
    created_at bigint NOT NULL CHECK (created_at > 0),
    updated_at bigint NOT NULL CHECK (updated_at >= created_at),
    archived_at bigint NOT NULL DEFAULT 0 CHECK (archived_at >= 0),
    restricted_at bigint NOT NULL DEFAULT 0 CHECK (restricted_at >= 0),
    restriction_reason text NOT NULL DEFAULT '',
    legacy_list_hidden boolean NOT NULL DEFAULT false
);
CREATE INDEX assistant_history_owner_page ON lmm_assistant.conversations(user_id, updated_at DESC, id DESC);
CREATE TABLE lmm_assistant.history_messages (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    conversation_id bigint NOT NULL REFERENCES lmm_assistant.conversations(id),
    sequence bigint NOT NULL CHECK (sequence > 0),
    role text NOT NULL CHECK (role IN ('user','assistant','human')),
    actor_user_id bigint CHECK (actor_user_id > 0),
    content text NOT NULL CHECK (char_length(content) BETWEEN 1 AND 4000),
    created_at bigint NOT NULL CHECK (created_at > 0),
    UNIQUE(conversation_id, sequence)
);
CREATE TABLE lmm_assistant.turn_receipts (
    user_id bigint NOT NULL CHECK (user_id > 0),
    client_turn_id text NOT NULL CHECK (length(client_turn_id) BETWEEN 1 AND 64),
    conversation_id bigint NOT NULL REFERENCES lmm_assistant.conversations(id),
    input_hash bytea NOT NULL CHECK (octet_length(input_hash)=32),
    answer_hash bytea NOT NULL CHECK (octet_length(answer_hash)=32),
    PRIMARY KEY(user_id, client_turn_id)
);
CREATE TABLE lmm_assistant.preset_stats (
    preset_id text PRIMARY KEY CHECK (preset_id IN ('ai_recommendation','getting_started','new_user_gift','weekly_discount')),
    clicks bigint NOT NULL DEFAULT 0 CHECK (clicks BETWEEN 0 AND 9007199254740991)
);
INSERT INTO lmm_assistant.preset_stats(preset_id) VALUES
    ('ai_recommendation'),('getting_started'),('new_user_gift'),('weekly_discount');

-- Offline installation/upgrade receipts; never granted to the runtime role.
CREATE TABLE lmm_assistant.history_upgrades (
    version integer PRIMARY KEY CHECK(version=3),
    operation_id text NOT NULL,
    database_oid oid NOT NULL,
    installed_by name NOT NULL
);
CREATE TABLE lmm_assistant.history_projections (
 batch text PRIMARY KEY,operation_id text NOT NULL,database_oid oid NOT NULL,installed_by name NOT NULL,
 conversations bigint NOT NULL,messages bigint NOT NULL,human bigint NOT NULL,hidden_shells bigint NOT NULL
);
