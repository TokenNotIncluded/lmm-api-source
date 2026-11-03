# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Offline source-bound, append-only owner receipts. No money is replayed."""
import re
from prepare_wallet_history_snapshot import literal, require


def owner_history_import_sql(manifest, *, scope, error_prefix):
    require(manifest.get("scope") == scope, "receipt_scope_mismatch")
    require(re.fullmatch("[a-z_]+", error_prefix) is not None, "receipt_error_prefix_invalid")
    return f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL standard_conforming_strings=on;
SET LOCAL lock_timeout='5s';
SET LOCAL statement_timeout='60s';
SELECT pg_advisory_xact_lock(741803121);
DO $owner_history$
DECLARE m jsonb := {literal(manifest)}::jsonb; previous core_migration.import_runs%ROWTYPE;
BEGIN
 IF NOT EXISTS(SELECT FROM pg_class WHERE oid='core_migration.import_runs'::regclass
   AND pg_get_userbyid(relowner)=current_user)
 THEN RAISE EXCEPTION '{error_prefix}_installation_owner_required'; END IF;
 IF NOT EXISTS(SELECT FROM core_billing.opening_import_runs
   WHERE source_archive_sha256=m->>'source_archive_sha256'
   AND observed_at=(m->>'observed_at')::bigint)
 THEN RAISE EXCEPTION '{error_prefix}_opening_source_mismatch'; END IF;
 IF EXISTS(SELECT FROM jsonb_array_elements_text(m->'source_user_ids') AS x(id)
   LEFT JOIN core_identity.users u ON u.id=x.id::bigint
   WHERE u.id IS NULL OR u.personal_account_id<>u.id)
 THEN RAISE EXCEPTION '{error_prefix}_identity_mapping_mismatch'; END IF;
 IF EXISTS(SELECT FROM core_migration.import_runs
   WHERE manifest->>'scope'={literal(scope)} AND migration_id<>m->>'migration_id')
 THEN RAISE EXCEPTION '{error_prefix}_other_source_installed'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.manifest IS DISTINCT FROM m
   OR previous.source_archive_sha256 IS DISTINCT FROM m->>'source_archive_sha256'
   OR previous.mapping_sha256 IS DISTINCT FROM m->>'mapping_sha256'
  THEN RAISE EXCEPTION '{error_prefix}_import_conflict'; END IF;
 ELSE
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',m);
 END IF;
END $owner_history$;
COMMIT;
"""

