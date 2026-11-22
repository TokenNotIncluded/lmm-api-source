-- Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
-- Historical awards are already part of opening balances. These records never
-- become intentions, commands or Core journals, and runtime cannot write them.
CREATE TABLE lmm_checkins.legacy_history_contract (
 singleton boolean PRIMARY KEY CHECK(singleton),
 version integer NOT NULL CHECK(version=1)
);
INSERT INTO lmm_checkins.legacy_history_contract VALUES(true,1);
CREATE TABLE lmm_checkins.legacy_history_imports (
 operation_id text PRIMARY KEY CHECK(octet_length(operation_id) BETWEEN 16 AND 80),
 input_sha256 text NOT NULL UNIQUE CHECK(input_sha256 ~ '^[0-9a-f]{64}$'),
 source_namespace_sha256 text NOT NULL CHECK(source_namespace_sha256 ~ '^[0-9a-f]{64}$'),
 manifest jsonb NOT NULL CHECK(jsonb_typeof(manifest)='object'),
 source_rows bigint NOT NULL CHECK(source_rows>=0),
 mapped_rows bigint NOT NULL CHECK(mapped_rows>=0),
 archived_rows bigint NOT NULL CHECK(archived_rows>=0),
 added_rows bigint NOT NULL CHECK(added_rows>=0 AND added_rows<=source_rows),
 installed_by name NOT NULL,
 database_oid oid NOT NULL,
 CHECK(mapped_rows+archived_rows=source_rows)
);
CREATE TABLE lmm_checkins.legacy_history (
 source_namespace_sha256 text NOT NULL CHECK(source_namespace_sha256 ~ '^[0-9a-f]{64}$'),
 source_id bigint NOT NULL CHECK(source_id>0),
 source_user_id bigint NOT NULL CHECK(source_user_id>0),
 day date NOT NULL CHECK(day BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
 amount_units bigint NOT NULL CHECK(amount_units>=0),
 created_at bigint NOT NULL CHECK(created_at>=0),
 target_user_id bigint CHECK(target_user_id>0),
 target_personal_account_id bigint CHECK(target_personal_account_id>0),
 source_row_sha256 text NOT NULL CHECK(source_row_sha256 ~ '^[0-9a-f]{64}$'),
 import_operation text NOT NULL REFERENCES lmm_checkins.legacy_history_imports(operation_id),
 PRIMARY KEY(source_namespace_sha256,source_id),
 UNIQUE(source_namespace_sha256,source_user_id,day),
 UNIQUE(target_user_id,day),
 UNIQUE(target_personal_account_id,day),
 CHECK((target_user_id IS NULL)=(target_personal_account_id IS NULL))
);
CREATE TRIGGER legacy_history_contract_immutable BEFORE UPDATE OR DELETE ON lmm_checkins.legacy_history_contract FOR EACH ROW EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER legacy_history_contract_no_truncate BEFORE TRUNCATE ON lmm_checkins.legacy_history_contract FOR EACH STATEMENT EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER legacy_history_imports_immutable BEFORE UPDATE OR DELETE ON lmm_checkins.legacy_history_imports FOR EACH ROW EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER legacy_history_imports_no_truncate BEFORE TRUNCATE ON lmm_checkins.legacy_history_imports FOR EACH STATEMENT EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER legacy_history_immutable BEFORE UPDATE OR DELETE ON lmm_checkins.legacy_history FOR EACH ROW EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER legacy_history_no_truncate BEFORE TRUNCATE ON lmm_checkins.legacy_history FOR EACH STATEMENT EXECUTE FUNCTION lmm_checkins.immutable();
REVOKE ALL ON lmm_checkins.legacy_history_contract,lmm_checkins.legacy_history_imports,lmm_checkins.legacy_history FROM PUBLIC;
