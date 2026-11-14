-- Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
-- Explicit offline v2 -> v3; existing rows and their authors remain unchanged.
ALTER TABLE lmm_assistant.conversations DROP CONSTRAINT conversations_title_check;
ALTER TABLE lmm_assistant.conversations DROP CONSTRAINT conversations_last_message_preview_check;
ALTER TABLE lmm_assistant.conversations ADD CONSTRAINT conversations_title_check CHECK(char_length(title) BETWEEN 0 AND 255);
ALTER TABLE lmm_assistant.conversations ADD CONSTRAINT conversations_last_message_preview_check CHECK(char_length(last_message_preview) BETWEEN 0 AND 512);
ALTER TABLE lmm_assistant.conversations ADD COLUMN restriction_reason text NOT NULL DEFAULT '';
ALTER TABLE lmm_assistant.conversations ADD COLUMN legacy_list_hidden boolean NOT NULL DEFAULT false;
ALTER TABLE lmm_assistant.history_messages DROP CONSTRAINT history_messages_role_check;
ALTER TABLE lmm_assistant.history_messages ADD CONSTRAINT history_messages_role_check CHECK(role IN('user','assistant','human'));
ALTER TABLE lmm_assistant.history_messages ADD COLUMN actor_user_id bigint CHECK(actor_user_id>0);
CREATE TABLE lmm_assistant.history_upgrades (
 version integer PRIMARY KEY CHECK(version=3),operation_id text NOT NULL,database_oid oid NOT NULL,installed_by name NOT NULL
);
REVOKE ALL ON lmm_assistant.history_upgrades FROM PUBLIC;
CREATE TABLE lmm_assistant.history_projections (
 batch text PRIMARY KEY,operation_id text NOT NULL,database_oid oid NOT NULL,installed_by name NOT NULL,
 conversations bigint NOT NULL,messages bigint NOT NULL,human bigint NOT NULL,hidden_shells bigint NOT NULL
);
REVOKE ALL ON lmm_assistant.history_projections FROM PUBLIC;
ALTER TABLE lmm_assistant.contract DROP CONSTRAINT contract_version_check;
UPDATE lmm_assistant.contract SET version=3 WHERE singleton AND version=2;
ALTER TABLE lmm_assistant.contract ADD CONSTRAINT contract_version_check CHECK(version=3);
