-- Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
-- Explicit owner-installed operations credential extension, independent of Core schema.
ALTER TABLE lmm_operations.script_repository ADD COLUMN github_key_ciphertext text NOT NULL DEFAULT ''
 CHECK (octet_length(github_key_ciphertext)<=8192);
CREATE TABLE lmm_operations.script_repository_credentials_upgrades (
 version integer PRIMARY KEY CHECK(version=1),
 operation_id text NOT NULL,
 database_oid oid NOT NULL,
 installed_by name NOT NULL
);
INSERT INTO lmm_operations.script_repository_credentials_upgrades
 SELECT 1,'fresh-install',oid,session_user FROM pg_database WHERE datname=current_database();
REVOKE ALL ON lmm_operations.script_repository_credentials_upgrades FROM PUBLIC;
