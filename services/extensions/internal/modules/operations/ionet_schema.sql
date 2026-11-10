-- Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
-- Independent extension records; no Core identity, funds or deployment authority.
CREATE TABLE lmm_operations.ionet_config (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
 revision bigint NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
 enabled boolean NOT NULL DEFAULT false,
 api_key_ciphertext text NOT NULL DEFAULT '' CHECK(octet_length(api_key_ciphertext)<=8192),
 updated_by bigint CHECK(updated_by BETWEEN 1 AND 9007199254740991),
 CHECK(NOT enabled OR api_key_ciphertext<>'')
);
INSERT INTO lmm_operations.ionet_config(singleton) VALUES(true);
CREATE TABLE lmm_operations.ionet_operations (
 actor_user_id bigint NOT NULL CHECK(actor_user_id BETWEEN 1 AND 9007199254740991),
 operation_key text NOT NULL CHECK(length(operation_key) BETWEEN 16 AND 128),
 kind text NOT NULL CHECK(kind IN ('create','extend','update','rename','delete','cancel')),
 resource_key text NOT NULL CHECK(octet_length(resource_key)<=300),
 deployment_id text NOT NULL DEFAULT '' CHECK(octet_length(deployment_id)<=128),
 request_sha256 text NOT NULL CHECK(request_sha256~'^[0-9a-f]{64}$'),
 config_revision bigint NOT NULL,
 state text NOT NULL CHECK(state IN ('unknown','succeeded','rejected','cancelled')),
 request_ciphertext text NOT NULL CHECK(octet_length(request_ciphertext)<=262144),
 result jsonb CHECK(result IS NULL OR octet_length(result::text)<=1048576),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(actor_user_id,operation_key)
);
-- A second key/user must not dispatch another mutation while the first outcome
-- is unknown. A timeout never releases this fence.
CREATE UNIQUE INDEX ionet_one_unknown_resource ON lmm_operations.ionet_operations(resource_key) WHERE state='unknown';
CREATE INDEX ionet_operations_actor_time ON lmm_operations.ionet_operations(actor_user_id,created_at DESC) WHERE state='unknown';
CREATE TABLE lmm_operations.ionet_upgrades (
 version integer PRIMARY KEY CHECK(version=1),operation_id text NOT NULL,database_oid oid NOT NULL,installed_by name NOT NULL
);
INSERT INTO lmm_operations.ionet_upgrades SELECT 1,'fresh-install',oid,session_user FROM pg_database WHERE datname=current_database();
REVOKE ALL ON lmm_operations.ionet_config,lmm_operations.ionet_operations,lmm_operations.ionet_upgrades FROM PUBLIC;
