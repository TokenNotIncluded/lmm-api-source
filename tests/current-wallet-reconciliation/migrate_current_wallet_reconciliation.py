#!/usr/bin/env python3
"""Offline, append-only current wallet/debt/affiliate reconciliation for Core78.

This is a one-epoch catch-up of an already-used target, not another opening of
its existing accounts. B is the actual original signed opening; D comprises
subsequent native entries. The existing native value must be N=B+D. The new
value is S+D, so the correction is S-B, never S-N. Reserved/escrow and all old
history remain intact. A previous different catch-up epoch is rejected rather
than silently counting its correction as new native activity.

Facts are rebuilt from the verified immutable current selected-source proof or
canonical export, actual opening receipts and ordered native ledger entries.
Historical units bind to the original export and retained original evidence;
current units come from the actual restored-source options. Producing a preview
never creates a review or approval. Apply requires separately supplied
source review and explicit authorization of the exact preview and SQL bytes.
The operator must obtain that authorization from the user; a file digest is a
binding to evidence, not proof of the user's identity or economic approval.

Only existing tables and triggers are used. Existing owners receive kind=
opening journals under this independent scope; new empty owners use the
existing import_opening_balance function. No SQL changes balance_state,
triggers, schemas, old receipts, old journals or old entries. Sequence values
are obtained normally and may leave gaps on rollback. Unknown commit outcomes
are resolved by reading the original instance/receipt; absence stays UNKNOWN.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import uuid

import migrate_current_runtime_keys as U
import legacy_routing_migration as R

F, I, C = U.F, U.I, U.C
SCOPE = "current-wallet-debt-affiliate-reconciliation-v1"
POLICY = "source-replacement-plus-preserved-native-delta"
SAFE = 9007199254740991
MAX = 9223372036854775807
MIN = -9223372036854775808
BUCKETS = ("wallet", "debt", "reserved", "affiliate")
CHANGED = ("core_billing.ledger_accounts", "core_billing.balance_state",
           "core_billing.ledger_operations", "core_billing.ledger_journals",
           "core_billing.ledger_entries", "core_billing.opening_import_runs", "core_billing.opening_imports")
SEQUENCES = ("core_billing.ledger_accounts_id_seq", "core_billing.ledger_journals_id_seq")
DECISION = "apply-reviewed-current-wallet-debt-affiliate-deltas"
OTHER_RIGHTS = "preserve-all-native-reserved-escrow-subscriptions-pending-rights-and-history"
literal, canonical, digest = U.literal, U.canonical, U.digest
Error, need = U.Error, U.need


def integer(value, code, minimum=MIN, maximum=MAX):
    need(type(value) is int and minimum <= value <= maximum, code)
    return value


def raw_sha(path):
    return hashlib.sha256(C.private_bytes(Path(path))).hexdigest()


def json_sha(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def selected_file_bytes(path):
    """Explicit private selected-source evidence, without changing old loaders.

    The source collector writes to the authorized private out directory. This
    new read-only format admits its owned 0600 files under an owned 0700 parent,
    with no path symlinks. The canonical loader and all execution/approval input
    paths retain their original BACKUP_ROOT checks.
    """
    p = Path(path)
    need(p.is_absolute() and p.resolve(strict=True) == p, "wallet_selected_path_not_explicit")
    I.owned_path(p.parent, private=True, directory=True)
    I.owned_path(p, private=True)
    fd = os.open(p, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        before = os.fstat(fd)
        need(stat.S_ISREG(before.st_mode) and before.st_uid == os.getuid() and before.st_mode & 0o077 == 0,
             "wallet_selected_file_not_private")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            value = stream.read()
        after = os.fstat(fd)
        need((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
             (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns),
             "wallet_selected_file_changed_during_read")
        return value
    finally:
        os.close(fd)


def selected_json(path):
    return json.loads(selected_file_bytes(path), object_pairs_hook=C.PROPOSAL.unique_object)


def selected_ref(path):
    return {"path": str(path), "sha256": hashlib.sha256(selected_file_bytes(path)).hexdigest(),
            "reader": "private-selected-source-v1"}


def selected_archive_ref(path):
    # Large immutable archives/exports are streamed, not read into memory.
    p = Path(path)
    need(p.is_absolute() and p.resolve(strict=True) == p, "wallet_archive_path_not_explicit")
    I.owned_path(p.parent, private=True, directory=True)
    I.owned_path(p, private=True)
    fd = os.open(p, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        before = os.fstat(fd)
        need(stat.S_ISREG(before.st_mode) and before.st_uid == os.getuid() and before.st_mode & 0o077 == 0,
             "wallet_archive_not_private")
        value = hashlib.sha256()
        with os.fdopen(fd, "rb", closefd=False) as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                value.update(chunk)
        after = os.fstat(fd)
        need((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
             (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns), "wallet_archive_changed")
        return {"path": str(p), "sha256": value.hexdigest(), "reader": "private-selected-archive-v1"}
    finally:
        os.close(fd)


def file_ref(path):
    path = Path(path).absolute()
    return {"path": str(path), "sha256": raw_sha(path)}


def verify_files(files):
    need(type(files) is dict and bool(files), "wallet_source_file_bindings_missing")
    for ref in files.values():
        need(type(ref) is dict and set(ref) in ({"path", "sha256"}, {"path", "sha256", "reader"}),
             "wallet_source_file_binding_invalid")
        reader = ref.get("reader")
        actual = (selected_ref(ref["path"])["sha256"] if reader == "private-selected-source-v1" else
                  selected_archive_ref(ref["path"])["sha256"] if reader == "private-selected-archive-v1" else
                  raw_sha(ref["path"]) if reader is None else None)
        need(actual == ref["sha256"], "wallet_source_file_changed")


def load_source(receipt_path, proof_path):
    """Existing all-table export verifier; retain only users/options in memory."""
    receipt, proof = C.load_json(Path(receipt_path)), C.load_json(Path(proof_path))
    hashes = {Path(receipt_path): raw_sha(receipt_path), Path(proof_path): raw_sha(proof_path)}
    tables, actual, _baseline = R.load_routing_export(receipt, Path(proof_path), hashes,
                                                    retained_tables=("users", "options"))
    need(proof == actual, "wallet_export_proof_changed")
    R.verify_files(hashes)
    return {"archive_sha256": proof["source_archive_sha256"], "export_sha256": proof["source_export_sha256"],
            "users": tables["users"], "options": tables["options"],
            "source_kind": "canonical-full-export", "observed_at": F.PLANNER.source_observation(receipt)["observed_at"],
            "files": {str(p): {"path": str(p), "sha256": h} for p, h in hashes.items()}}


def load_selected_source(proof_path):
    proof = selected_json(proof_path)
    need(proof.get("format") == "lmm.selected-source-facts-proof.v1" and proof.get("scope") ==
         "SELECTED_FIELDS_ONLY_NOT_CANONICAL_FINANCIAL_EXPORT_NOT_APPROVAL"
         and proof.get("actual_source_selected_query_exit") == 0
         and proof.get("original_source_columns_complete") is False
         and proof.get("financial_approval") is False, "wallet_selected_source_contract_invalid")
    files = {"selected_proof": selected_ref(proof_path)}
    for field in ("source_sql", "source_selected_file", "execution_assertion_producer"):
        ref = selected_ref(proof[field + "_path"])
        need(ref["sha256"] == proof[field + "_sha256"], "wallet_selected_source_file_mismatch")
        files[field] = ref
    payload = selected_json(proof["source_selected_file_path"])
    need(set(payload) == {"bindings", "source"}, "wallet_selected_payload_invalid")
    bindings, source = payload["bindings"], payload["source"]
    need(json_sha(source) == proof["source_selected_inventory_sha256"] == bindings["source_selected_inventory_sha256"],
         "wallet_selected_semantic_digest_mismatch")
    need(bindings["fresh_archive_sha256"] == proof["fresh_archive_sha256"]
         and bindings["restore_receipt_sha256"] == proof["restore_receipt_sha256"]
         and bindings["source_sql_sha256"] == proof["source_sql_sha256"], "wallet_selected_binding_mismatch")
    restore_ref = selected_ref(bindings["restore_receipt_path"])
    need(restore_ref["sha256"] == proof["restore_receipt_sha256"], "wallet_restore_receipt_changed")
    files["restore_receipt"] = restore_ref
    restore = selected_json(bindings["restore_receipt_path"])
    need(restore.get("status") == "RESTORED_SOURCE_ONLY_READONLY_PREVIEW_NOT_FINANCIAL_APPROVAL"
         and restore.get("restore_actual_exit") == 0 and restore.get("original_archive_preserved") is True
         and restore.get("production_writes") == 0 and restore.get("target_database_accessed_by_restore") is False
         and restore.get("financial_approval") is False
         and restore["production_source_observation"].get("not_production_dump_snapshot_identity") is True,
         "wallet_actual_source_restore_unproven")
    archive = selected_archive_ref(bindings["fresh_archive_path"])
    need(archive["sha256"] == proof["fresh_archive_sha256"] == restore["archive"]["sha256"]
         and restore["archive"]["path"] == bindings["fresh_archive_path"], "wallet_restored_archive_mismatch")
    files["selected_archive"] = archive
    local = proof["local_source_identity"]
    ri, rc, rp = restore["restored_instance"], restore["connection"], restore["readonly_reader_proof"]
    need(local == {k: source[k] for k in ("database", "nonce", "observed_at", "local_snapshot")}
         and local["database"] == ri["database"] == rc["database"] and local["nonce"] == ri["nonce"]
         and rc["host"] == ri["socket_directory"] and str(rc["port"]) == str(ri["port"])
         and ri["listen_addresses"] == "" and ri["transaction_read_only"] == "on"
         and ri["system_identifier"] != restore["production_source_observation"]["system_identifier"],
         "wallet_selected_restore_instance_mismatch")
    observed_namespace = {k: restore["production_source_observation"][k]
                          for k in ("system_identifier", "database", "schema")}
    U.namespace_key(observed_namespace)
    need(rp["read_only"] == "on" and rp["row_security"] == "off"
         and set(rp["role_flags"]) == {"superuser", "createrole", "createdb", "replication", "bypassrls"}
         and all(v is False for v in rp["role_flags"].values()) and rp["role_memberships"] == 0
         and rp["database_create"] is False and rp["database_temp"] is False
         and all(rp[k] == 0 for k in ("application_table_write_privileges", "application_schema_create_privileges",
                                    "application_security_definer_execute", "rls_tables")),
         "wallet_source_reader_not_readonly")
    fields = {"id", "quota", "affiliate", "role", "status", "group", "created_at", "deleted_at",
              "source_identity_facts_sha256", "source_selected_facts_sha256"}
    rows, seen = [], set()
    for row in source["users"]:
        need(set(row) == fields, "wallet_selected_user_shape_invalid")
        uid = integer(row["id"], "wallet_selected_user_id_invalid", 1)
        for field in ("quota", "affiliate", "role", "status", "created_at"):
            integer(row[field], "wallet_selected_scalar_type_invalid")
        need(type(row["group"]) is str and (row["deleted_at"] is None or type(row["deleted_at"]) is str),
             "wallet_selected_scalar_type_invalid")
        need(uid not in seen, "wallet_selected_user_duplicate")
        seen.add(uid)
        raw = {k: v for k, v in row.items() if k not in {"source_identity_facts_sha256", "source_selected_facts_sha256"}}
        need(json_sha(raw) == row["source_selected_facts_sha256"], "wallet_selected_user_digest_mismatch")
        # The identity-facts digest cannot be recomputed from this redacted
        # payload. Retain it only as a binding; never claim full identity proof.
        F.checked_hash(row["source_identity_facts_sha256"])
        rows.append({"id": uid, "quota": row["quota"], "aff_quota": row["affiliate"],
                     "selected_facts": raw, "source_selected_facts_sha256": row["source_selected_facts_sha256"]})
    need(len(rows) == proof["selected_user_count"] == proof["restored_baseline_users"]
         and proof["all_source_user_ids_unique"] is True and proof["unit_options_exact_rows"] == source["unit_options"],
         "wallet_selected_source_coverage_unproven")
    # The restore collector's baseline is another actual file, not the count
    # in an approval statement. Keep its exact bytes within the proof closure.
    files["restored_table_baseline"] = selected_ref(restore["table_baseline_path"])
    baseline = selected_json(restore["table_baseline_path"])
    schema = restore["production_source_observation"]["schema"]
    baseline_users = [r for r in baseline["tables"] if r["schema"] == schema and r["table"] == "users"]
    need(len(baseline["tables"]) == restore["table_baseline_count"] and len(baseline_users) == 1
         and integer(baseline_users[0]["rows"], "wallet_baseline_users_invalid", 0) == len(rows),
         "wallet_selected_actual_baseline_count_mismatch")
    observed = datetime.fromisoformat(source["observed_at"].replace("Z", "+00:00"))
    need(observed.tzinfo is not None and observed.utcoffset().total_seconds() == 0,
         "wallet_selected_observation_not_utc_bound")
    return {"archive_sha256": archive["sha256"], "export_sha256": proof["source_selected_file_sha256"],
        "source_kind": "selected-wallet-source-facts-v1", "users": rows, "options": source["unit_options"],
        "observed_at": int(observed.timestamp()), "source_production_fenced": proof["source_production_fenced"],
        "selected_proof": proof, "restore_system_identifier": ri["system_identifier"],
        "source_namespace_observation": observed_namespace, "files": files}


def load_committed_baseline(spec):
    evidence = selected_json(spec["unit_evidence"])
    need(evidence.get("scope") == "actual-unit-evidence-not-approval"
         and evidence.get("source_financial_plan_unit") == "credit_500k_usd"
         and evidence.get("source_financial_plan_credits_per_usd") == 500000
         and evidence.get("no_fx_conversion_requested") is True
         and evidence.get("source_fixed_contract") == "const FixedCreditsPerUSD int64 = 500000",
         "wallet_original_unit_evidence_contract_invalid")
    raw = selected_archive_ref(evidence["source_raw_export"])
    version = selected_ref(evidence["source_version_evidence"])
    files = {"baseline_unit_" + spec["source_archive_sha256"]: selected_ref(spec["unit_evidence"]),
             "baseline_export_" + spec["source_archive_sha256"]: raw,
             "baseline_version_" + spec["source_archive_sha256"]: version}
    return {"source_kind": "committed-opening-baseline", "archive_sha256": spec["source_archive_sha256"],
            "export_sha256": raw["sha256"], "options": evidence["observed_options"],
            "unit_evidence": evidence, "files": files}


def load_sources(bundle_path):
    bundle = C.load_json(Path(bundle_path))
    need(set(bundle) == {"format_version", "current", "baselines"} and bundle["format_version"] == 1,
         "wallet_source_bundle_shape_invalid")
    def source(x):
        if x.get("kind") == "selected-wallet-source-facts-v1":
            return load_selected_source(x["selected_source_proof"])
        if x.get("kind") == "committed-opening-baseline":
            return load_committed_baseline(x)
        return load_source(x["source_receipt"], x["export_proof"])
    current = source(bundle["current"])
    baselines = [source(x) for x in bundle["baselines"]]
    files = {"source_bundle": file_ref(bundle_path), **current["files"]}
    for baseline in baselines:
        files.update(baseline["files"])
    need(len({s["archive_sha256"] for s in baselines}) == len(baselines), "wallet_duplicate_baseline_archive")
    return current, baselines, files


def unit_basis(source):
    # Full/selected sources carry actually exported options. The explicit
    # committed-baseline profile instead binds the retained historical unit
    # observation to the same immutable raw export and native opening material;
    # it does not claim a new complete historical row rescan.
    rows = [r for r in source["options"] if r.get("key") in ("CreditsPerUSD", "QuotaPerUnit")]
    need(len(rows) == 2 and {r["key"]: r["value"] for r in rows} ==
         {"CreditsPerUSD": "500000", "QuotaPerUnit": "500000"}, "wallet_snapshot_unit_options_unproven")
    return {"source_archive_sha256": source["archive_sha256"], "source_export_sha256": source["export_sha256"],
            "unit": "credit_500k_usd", "credits_per_usd": 500000, "numerator": 1, "denominator": 1,
            "observed_options": sorted(rows, key=lambda r: r["key"]), "options_sha256": digest(source["options"]),
            "basis": source.get("source_kind", "synthetic-full-export")}


def ledger_facts_sql():
    return """WITH ordered AS (
 SELECT e.*,sum(e.delta_units::numeric) OVER(PARTITION BY e.ledger_account_id ORDER BY e.balance_revision) running,
 row_number() OVER(PARTITION BY e.ledger_account_id ORDER BY e.balance_revision) ordinal
 FROM core_billing.ledger_entries e
), stats AS (
 SELECT ledger_account_id,sum(delta_units::numeric) total,count(*) n,
 bool_and(running=balance_after_units AND ordinal=balance_revision) valid FROM ordered GROUP BY ledger_account_id
)
SELECT jsonb_build_object(
 'opening_manifests',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.migration_id),'[]'::jsonb)
  FROM core_migration.import_runs r WHERE EXISTS(SELECT 1 FROM core_billing.opening_imports o WHERE o.migration_id=r.migration_id)),
 'owners',(SELECT coalesce(jsonb_agg(jsonb_build_object('account_id',u.personal_account_id,'user_id',u.id,
  'opening',(SELECT to_jsonb(o) FROM core_billing.opening_imports o WHERE o.account_id=u.personal_account_id),
  'opening_run',(SELECT to_jsonb(r) FROM core_billing.opening_import_runs r JOIN core_billing.opening_imports o
   ON o.migration_id=r.migration_id WHERE o.account_id=u.personal_account_id),
  'buckets',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',a.id,'bucket',a.bucket,'unit',a.unit,
   'units',b.balance_units,'revision',b.revision,'entries_sum',coalesce(s.total,0),'entries_count',coalesce(s.n,0),
   'path_valid',coalesce(s.valid,true),'opening_units',coalesce((SELECT sum(e.delta_units::numeric)
   FROM core_billing.ledger_entries e JOIN core_billing.opening_imports o ON (o.result->>'journal_id')::bigint=e.journal_id
   WHERE e.ledger_account_id=a.id AND o.account_id=u.personal_account_id),0)) ORDER BY a.bucket),'[]'::jsonb)
   FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
   LEFT JOIN stats s ON s.ledger_account_id=a.id WHERE a.owner_account_id=u.personal_account_id))
  ORDER BY u.personal_account_id),'[]'::jsonb) FROM core_identity.users u),
 'clearing',(SELECT jsonb_build_object('id',a.id,'units',b.balance_units,'revision',b.revision,
  'entries_sum',coalesce(s.total,0),'entries_count',coalesce(s.n,0),'path_valid',coalesce(s.valid,true))
  FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
  LEFT JOIN stats s ON s.ledger_account_id=a.id WHERE a.bucket='clearing'),
 'global_valid',NOT EXISTS(SELECT 1 FROM core_billing.balance_state b LEFT JOIN stats s ON s.ledger_account_id=b.ledger_account_id
  WHERE b.balance_units::numeric<>coalesce(s.total,0) OR b.revision<>coalesce(s.n,0) OR NOT coalesce(s.valid,true))
  AND NOT EXISTS(SELECT 1 FROM core_billing.ledger_journals j LEFT JOIN core_billing.ledger_entries e ON e.journal_id=j.id
   GROUP BY j.id HAVING count(e.journal_id)<2 OR sum(e.delta_units::numeric)<>0)
  AND (SELECT coalesce(sum(balance_units::numeric),0) FROM core_billing.balance_state)=0,
 'max_account_id',(SELECT coalesce(max(id),0) FROM core_billing.ledger_accounts),
 'max_journal_id',(SELECT coalesce(max(id),0) FROM core_billing.ledger_journals))"""


def snapshot_sql(contract, inventory, migration_id):
    # Append facts to the same repeatable-read snapshot produced by the frozen
    # used-target helper. The exact ending is a checked interface boundary.
    base = U.snapshot_sql(contract, inventory, migration_id)
    suffix = ");\nCOMMIT;\n"
    need(base.endswith(suffix), "wallet_snapshot_helper_interface_changed")
    return base[:-len(suffix)] + ") || jsonb_build_object('ledger',(" + ledger_facts_sql() + ")," + \
        "'reconciliation_runs',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) " + \
        "FROM core_migration.import_runs r WHERE manifest->>'scope'=" + literal(SCOPE) + "));\nCOMMIT;\n"


def validate_path(bucket):
    integer(bucket["units"], "wallet_native_units_out_of_range")
    integer(bucket["revision"], "wallet_native_revision_invalid", 0)
    need(bucket["path_valid"] is True and bucket["entries_sum"] == bucket["units"]
         and bucket["entries_count"] == bucket["revision"], "wallet_native_ledger_path_invalid")


def source_rows(source):
    result = {}
    for row in source["users"]:
        uid = integer(row.get("id"), "wallet_source_user_invalid", 1)
        need(uid not in result, "wallet_source_user_duplicate")
        result[uid] = row
    return result


def position(source_row, mapping, fact, baselines):
    uid, account = mapping["source_user_id"], mapping["target_personal_account_id"]
    need(fact["user_id"] == mapping["target_user_id"] and fact["account_id"] == account,
         "wallet_native_owner_mismatch")
    s = integer(source_row["quota"], "wallet_source_quota_invalid", -SAFE, SAFE)
    sa = integer(source_row["aff_quota"], "wallet_source_affiliate_invalid", -SAFE, SAFE)
    buckets = {x["bucket"]: x for x in fact["buckets"]}
    need(len(buckets) == len(fact["buckets"]) and set(buckets) in (set(), set(BUCKETS)),
         "wallet_native_bucket_set_incomplete")
    for bucket in buckets.values():
        need(bucket["unit"] == "credit_500k_usd", "wallet_native_unit_mismatch")
        validate_path(bucket)
    initial = fact["opening"] is None
    if initial:
        need(fact["opening_run"] is None and all(x["units"] == x["revision"] == 0 for x in buckets.values()),
             "wallet_new_owner_not_empty")
        bw = ba = br = 0
        anchor = None
    else:
        opening, run = fact["opening"], fact["opening_run"]
        need(set(buckets) == set(BUCKETS) and type(run) is dict, "wallet_opening_receipt_missing")
        request = opening["request"]
        need(opening["source_entity_key"] == request["source_entity_key"] == "legacy.users/" + str(uid)
             and opening["account_id"] == request["account_id"] == account
             and opening["migration_id"] == run["migration_id"] == request["migration_id"]
             and run["source_archive_sha256"] == request["source_archive_sha256"]
             and run["mapping_sha256"] == request["mapping_sha256"]
             and run["observed_at"] == request["observed_at"], "wallet_opening_receipt_binding_invalid")
        source = baselines.get(run["source_archive_sha256"])
        need(source is not None, "wallet_historical_source_proof_missing")
        bw = integer(request["available_units"], "wallet_opening_wallet_invalid", -SAFE, SAFE)
        ba = integer(request["affiliate_units"], "wallet_opening_affiliate_invalid", -SAFE, SAFE)
        br = integer(request["reserved_units"], "wallet_opening_reserved_invalid", 0, SAFE)
        if source.get("source_kind") == "committed-opening-baseline":
            native_manifest = source["native_manifests"].get(opening["migration_id"])
            need(type(native_manifest) is dict and native_manifest["source_archive_sha256"] == run["source_archive_sha256"]
                 and native_manifest["mapping_sha256"] == run["mapping_sha256"], "wallet_original_native_manifest_missing")
            manifest = native_manifest["manifest"]
            old_material = manifest.get("material", {})
            need(manifest.get("scope") == old_material.get("scope") == "financial-available-wallet-debt-v1"
                 and old_material.get("policy") == "original-signed-users-quota-only-no-rescale-all-other-rights-explicitly-deferred"
                 and old_material.get("source_archive_sha256") == run["source_archive_sha256"]
                 and old_material.get("source_files", {}).get("source_export", {}).get("sha256") == source["export_sha256"],
                 "wallet_original_partial_opening_contract_mismatch")
            core = {k: old_material[k] for k in ("scope", "policy", "source_archive_sha256", "source_plan_mapping_sha256", "positions")}
            need(json_sha(core) == old_material["mapping_sha256"] == run["mapping_sha256"], "wallet_original_partial_mapping_changed")
            candidates = [x for x in old_material["positions"] if x["source_user_id"] == uid]
            need(len(candidates) == 1 and candidates[0]["account_id"] == account
                 and candidates[0]["target_user_id"] == mapping["target_user_id"]
                 and candidates[0]["available_units"] == bw and ba == 0 and br == 0
                 and candidates[0]["source_row_sha256"] == opening["source_row_sha256"] == request["source_row_sha256"],
                 "wallet_committed_opening_position_mismatch")
        else:
            old = source_rows(source).get(uid)
            need(old is not None and digest(old) == opening["source_row_sha256"] == request["source_row_sha256"],
                 "wallet_historical_source_row_mismatch")
            need(bw == old["quota"], "wallet_opening_not_original_source_quota")
            need(ba == 0 or ba == old["aff_quota"], "wallet_affiliate_opening_unexplained")
        expected = {"wallet": max(bw, 0), "debt": min(bw, 0), "reserved": br, "affiliate": ba}
        need(all(buckets[k]["opening_units"] == expected[k] for k in BUCKETS),
             "wallet_opening_entries_do_not_match_receipt")
        anchor = {"opening": opening, "run": run, "source_export_sha256": source["export_sha256"]}
    native = {k: buckets[k]["units"] if k in buckets else 0 for k in BUCKETS}
    need(native["wallet"] >= 0 and native["debt"] <= 0 and native["reserved"] >= 0,
         "wallet_native_bucket_sign_invalid")
    n = native["wallet"] + native["debt"]
    need(native["wallet"] == max(n, 0) and native["debt"] == min(n, 0),
         "wallet_native_split_requires_explicit_reconciliation")
    dw = n - bw
    da = native["affiliate"] - ba
    target = {"wallet": max(s + dw, 0), "debt": min(s + dw, 0),
              "affiliate": sa + da, "reserved": native["reserved"]}
    for amount in target.values():
        integer(amount, "wallet_target_out_of_range")
    legs = {k: target[k] - native[k] for k in ("wallet", "debt", "affiliate")}
    for amount in legs.values():
        integer(amount, "wallet_delta_out_of_range")
    need(legs["wallet"] + legs["debt"] == s - bw and legs["affiliate"] == sa - ba,
         "wallet_delta_equation_failed")
    legs["clearing"] = integer(-sum(legs.values()), "wallet_clearing_delta_overflow")
    amount = integer(sum(abs(legs[k]) for k in ("wallet", "debt", "affiliate")), "wallet_journal_amount_overflow", 0)
    need(sum(v != 0 for v in legs.values()) >= 2 or amount == 0, "wallet_single_leg_journal")
    return {"source_user_id": uid, "target_user_id": mapping["target_user_id"], "account_id": account,
            "source_row_sha256": source_row.get("source_selected_facts_sha256", digest(source_row)),
            "source_row_contract": "selected-wallet-source-facts-v1" if "source_selected_facts_sha256" in source_row else "full-source-user-row",
            "initial": initial, "anchor": anchor,
            "source_S": s, "source_affiliate": sa, "opening_B": bw, "opening_affiliate_B": ba,
            "native_N": n, "native_D": dw, "native_affiliate_D": da,
            "before_buckets": buckets, "target_buckets": target, "legs": legs, "journal_amount": amount}


def prepare(current, historical, mappings, snapshot, contract, namespace, migration_id, files, runtime_roles):
    U.validate_snapshot(snapshot, contract)
    need(I.MIGRATION_ID.fullmatch(migration_id) and len(migration_id) <= 118, "wallet_migration_id_invalid")
    need(type(runtime_roles) is list and runtime_roles and runtime_roles == sorted(set(runtime_roles))
         and all(U.IDENTIFIER.fullmatch(r) for r in runtime_roles), "wallet_runtime_roles_invalid")
    ns = U.namespace_key(namespace)
    if current.get("source_kind") == "selected-wallet-source-facts-v1":
        need(current["source_namespace_observation"] == namespace, "wallet_source_namespace_observation_mismatch")
    need(snapshot["ledger"]["global_valid"] is True, "wallet_global_ledger_invalid")
    validate_path(snapshot["ledger"]["clearing"])
    native_manifests = {r["migration_id"]: r for r in snapshot["ledger"].get("opening_manifests", [])}
    baselines = {s["archive_sha256"]: {**s, "native_manifests": native_manifests} for s in historical}
    need(len(baselines) == len(historical), "wallet_duplicate_baseline_archive")
    units, blockers = [], []
    for source in [current, *historical]:
        try:
            units.append(unit_basis(source))
        except Error as exc:
            blockers.append({"code": str(exc), "source_archive_sha256": source["archive_sha256"]})
    current_rows = source_rows(current)
    native = {r["account_id"]: r for r in snapshot["ledger"]["owners"]}
    map_by_id, target_ids, accounts = {}, set(), set()
    for mapping in mappings:
        need(set(mapping) == {"source_user_id", "target_user_id", "target_personal_account_id"},
             "wallet_identity_map_shape_invalid")
        for n in mapping.values():
            integer(n, "wallet_identity_map_id_invalid", 1)
        uid, target, account = (mapping[k] for k in ("source_user_id", "target_user_id", "target_personal_account_id"))
        need(uid not in map_by_id and target not in target_ids and account not in accounts,
             "wallet_duplicate_identity_mapping")
        map_by_id[uid] = mapping
        target_ids.add(target)
        accounts.add(account)
    need(set(map_by_id) <= set(current_rows), "wallet_identity_map_has_foreign_source_users")
    positions = []
    for uid in sorted(current_rows):
        if uid not in map_by_id:
            blockers.append({"code": "wallet_source_owner_unmapped", "source_user_id": uid})
            continue
        mapping = map_by_id[uid]
        if mapping["target_personal_account_id"] not in native:
            blockers.append({"code": "wallet_target_owner_absent", "source_user_id": uid})
            continue
        try:
            positions.append(position(current_rows[uid], mapping, native[mapping["target_personal_account_id"]], baselines))
        except Error as exc:
            blockers.append({"code": str(exc), "source_user_id": uid})
    # Preserve one clearly bounded epoch. A second source replacement needs a
    # separately reviewed anchor, not treatment of this correction as native D.
    for prior in snapshot["reconciliation_runs"]:
        previous_accounts = {p["account_id"] for p in prior["manifest"].get("material", {}).get("positions", [])}
        if prior["manifest"].get("source_namespace_sha256") == ns or previous_accounts & accounts:
            blockers.append({"code": "wallet_source_already_reconciled", "migration_id": prior["migration_id"]})
    positions.sort(key=lambda r: r["account_id"])
    clearing = snapshot["ledger"]["clearing"]["units"]
    for row in positions:
        clearing = integer(clearing + row["legs"]["clearing"], "wallet_intermediate_clearing_overflow")
        row["clearing_after"] = clearing
    for name in SEQUENCES:
        need(name in snapshot["state"]["sequences"], "wallet_sequence_missing")
        first = C._next_id(snapshot["state"], name, 4 * len(positions) + 1)
        max_id = snapshot["ledger"]["max_account_id" if name == SEQUENCES[0] else "max_journal_id"]
        need(first > max_id, "wallet_sequence_behind_existing_rows")
    material = {"format_version": 1, "scope": SCOPE, "policy": POLICY, "migration_id": migration_id,
        "source_namespace": namespace, "source_namespace_sha256": ns,
        "source_archive_sha256": current["archive_sha256"], "source_export_sha256": current["export_sha256"],
        "source_kind": current.get("source_kind", "synthetic-full-export"),
        "source_wallet_facts_sha256": json_sha([{k: r[k] for k in ("id", "quota", "aff_quota")}
                                                for r in sorted(current_rows.values(), key=lambda r: r["id"])]),
        "source_production_fenced_at_preview": current.get("source_production_fenced", False),
        "source_observed_at": current.get("observed_at"),
        "source_users": len(current_rows), "identity_map": mappings, "identity_map_sha256": digest(mappings),
        "positions": positions, "units": units, "blockers": blockers, "ready_for_review": not blockers,
        "snapshot": snapshot, "before_state": snapshot["state"], "contract": contract,
        "runtime_roles": runtime_roles, "source_files": files, "other_rights_policy": OTHER_RIGHTS,
        "full_financial_import_complete": False,
        "tool_source_hashes": {**U.tool_sources(), Path(__file__).name: hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}}
    material["mapping_sha256"] = digest(positions)
    return material


def expected_balances(m):
    result = {}
    clearing = m["snapshot"]["ledger"]["clearing"]
    result[str(clearing["id"])] = {"before_units": clearing["units"], "before_revision": clearing["revision"],
        "units": clearing["units"], "revision": clearing["revision"]}
    for row in m["positions"]:
        for bucket, before in row["before_buckets"].items():
            delta = row["legs"].get(bucket, 0)
            result[str(before["id"])] = {"before_units": before["units"], "before_revision": before["revision"],
                "units": row["target_buckets"][bucket], "revision": before["revision"] + int(delta != 0)}
        if row["legs"]["clearing"]:
            result[str(clearing["id"])]["units"] += row["legs"]["clearing"]
            result[str(clearing["id"])]["revision"] += 1
    return result


def operation_key(m, row):
    return digest({"source_namespace_sha256": m["source_namespace_sha256"],
                   "source_export_sha256": m["source_export_sha256"], "source_user_id": row["source_user_id"]})


def initial_payload(m, row):
    return {"migration_id": m["migration_id"] + "/initial", "source_entity_key": "legacy.users/" + str(row["source_user_id"]),
        "source_archive_sha256": m["source_archive_sha256"], "mapping_sha256": m["mapping_sha256"],
        "source_row_sha256": row["source_row_sha256"], "account_id": row["account_id"],
        "available_units": row["source_S"], "affiliate_units": row["source_affiliate"], "reserved_units": 0,
        "observed_at": m["source_observed_at"]}


def preservation_sql(m):
    """Hash all old rows, projecting only trigger-updated balance fields back."""
    facts, inventory = m["snapshot"]["ledger"], m["before_state"]["inventory"]
    initial = [row for row in m["positions"] if row["initial"]]
    keys = ",".join(literal(operation_key(m, r)) for r in m["positions"]) or "NULL"
    initial_keys = ",".join("encode(sha256(convert_to(jsonb_build_array(" + literal(m["migration_id"] + "/initial") +
        "," + literal("legacy.users/" + str(r["source_user_id"])) + ")::text,'UTF8')),'hex')" for r in initial) or "NULL"
    wheres = {
        "core_billing.ledger_accounts": f"t.id<={facts['max_account_id']}",
        "core_billing.balance_state": f"t.ledger_account_id<={facts['max_account_id']}",
        "core_billing.ledger_journals": f"t.id<={facts['max_journal_id']}",
        "core_billing.ledger_entries": f"t.journal_id<={facts['max_journal_id']}",
        "core_billing.ledger_operations": f"NOT(t.scope={literal(SCOPE)} AND t.operation_key IN ({keys})) " +
            f"AND NOT(t.scope='offline-opening' AND t.operation_key IN ({initial_keys}))",
        "core_billing.opening_import_runs": "t.migration_id<>" + literal(m["migration_id"] + "/initial"),
        "core_billing.opening_imports": "t.migration_id<>" + literal(m["migration_id"] + "/initial"),
        "core_migration.import_runs": "t.migration_id<>" + literal(m["migration_id"])}
    expected = expected_balances(m)
    restore = literal(expected) + "::jsonb->t.ledger_account_id::text"
    balance_projection = f"CASE WHEN ({restore}) IS NULL THEN to_jsonb(t) ELSE to_jsonb(t)||" + \
        f"jsonb_build_object('balance_units',({restore}->>'before_units')::bigint," + \
        f"'revision',({restore}->>'before_revision')::bigint) END"
    selects = []
    for name in inventory["tables"]:
        projection = balance_projection if name == "core_billing.balance_state" else "to_jsonb(t)"
        where = " WHERE " + wheres[name] if name in wheres else ""
        selects.append(f"SELECT {literal(name)} name,jsonb_build_object('rows',count(*),'sha256'," +
            "encode(sha256(coalesce(string_agg(sha256(convert_to(v::text,'UTF8')),''::bytea ORDER BY " +
            "convert_to(v::text,'UTF8')),''::bytea)),'hex')) value FROM (SELECT " + projection +
            " v FROM " + F.relation(name) + " t" + where + ") old_rows")
    return "SELECT jsonb_object_agg(name,value) FROM (" + " UNION ALL ".join(selects) + ") old_tables"


def _readback_sql(m, *, current_balances=True):
    checks = []
    for row in m["positions"]:
        aid = row["account_id"]
        if current_balances:
            for bucket, units in row["target_buckets"].items():
                checks.append(f"IF NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b " +
                    f"ON b.ledger_account_id=a.id WHERE a.owner_account_id={aid} AND a.bucket={literal(bucket)} " +
                    f"AND b.balance_units={units}) THEN RAISE EXCEPTION 'wallet_bucket_readback_mismatch'; END IF;")
        if row["initial"]:
            checks.append("IF NOT EXISTS(SELECT 1 FROM core_billing.opening_imports WHERE request=" +
                literal(initial_payload(m, row)) + "::jsonb) THEN RAISE EXCEPTION 'wallet_initial_readback_mismatch'; END IF;")
        elif row["journal_amount"]:
            key = literal(operation_key(m, row))
            checks.append(f"IF NOT EXISTS(SELECT 1 FROM core_billing.ledger_operations o JOIN core_billing.ledger_journals j " +
                f"ON j.scope=o.scope AND j.operation_key=o.operation_key WHERE o.scope={literal(SCOPE)} " +
                f"AND o.operation_key={key} AND o.request={literal(row)}::jsonb AND o.result->>'status'='posted' " +
                f"AND (o.result->>'journal_id')::bigint=j.id AND j.kind='opening' AND j.amount_units={row['journal_amount']}) " +
                "THEN RAISE EXCEPTION 'wallet_operation_readback_mismatch'; END IF;")
            for bucket, delta in row["legs"].items():
                owner = "a.owner_account_id IS NULL" if bucket == "clearing" else f"a.owner_account_id={aid}"
                checks.append(f"IF coalesce((SELECT sum(e.delta_units::numeric) FROM core_billing.ledger_journals j " +
                    "JOIN core_billing.ledger_entries e ON e.journal_id=j.id JOIN core_billing.ledger_accounts a " +
                    f"ON a.id=e.ledger_account_id WHERE j.scope={literal(SCOPE)} AND j.operation_key={key} " +
                    f"AND a.bucket={literal(bucket)} AND {owner}),0)<>{delta} " +
                    "THEN RAISE EXCEPTION 'wallet_entry_readback_mismatch'; END IF;")
    return "\n".join(checks)


def _write_sql(m):
    writes = []
    for row in m["positions"]:
        if row["initial"]:
            writes.append("PERFORM core_billing.import_opening_balance(" + literal(initial_payload(m, row)) + "::jsonb);")
        elif row["journal_amount"]:
            key = literal(operation_key(m, row))
            ids = {bucket: value["id"] for bucket, value in row["before_buckets"].items()}
            ids["clearing"] = m["snapshot"]["ledger"]["clearing"]["id"]
            writes.append(f"INSERT INTO core_billing.ledger_journals(scope,operation_key,kind,payer_id,payee_id," +
                "amount_units,reason) VALUES(" + literal(SCOPE) + f",{key},'opening',{ids['clearing']},{ids['wallet']}," +
                f"{row['journal_amount']},'Offline current source wallet/debt/affiliate reconciliation; original history retained') " +
                "RETURNING id INTO v_journal_id;")
            for bucket in sorted(ids, key=lambda k: ids[k]):
                delta = row["legs"].get(bucket, 0)
                if delta:
                    writes.append("INSERT INTO core_billing.ledger_entries(journal_id,ledger_account_id,delta_units) " +
                                  f"VALUES(v_journal_id,{ids[bucket]},{delta});")
            writes.append("INSERT INTO core_billing.ledger_operations(scope,operation_key,request,result) VALUES(" +
                literal(SCOPE) + f",{key},{literal(row)}::jsonb,jsonb_build_object('status','posted','journal_id',v_journal_id));")
    return "\n".join(writes)


def validate_preview(m):
    need(m.get("format_version") == 1 and m.get("scope") == SCOPE and m.get("policy") == POLICY,
         "wallet_preview_contract_invalid")
    need(m.get("blockers") == [] and m.get("ready_for_review") is True
         and len(m["positions"]) == m["source_users"], "wallet_preview_has_unresolved_facts")
    need(m["mapping_sha256"] == digest(m["positions"]) and m["identity_map_sha256"] == digest(m["identity_map"]),
         "wallet_preview_binding_changed")
    need(m["other_rights_policy"] == OTHER_RIGHTS and m["full_financial_import_complete"] is False,
         "wallet_scope_expanded")
    need(m["tool_source_hashes"] == {**U.tool_sources(), Path(__file__).name: hashlib.sha256(Path(__file__).read_bytes()).hexdigest()},
         "wallet_writer_source_changed")
    U.validate_snapshot(m["snapshot"], m["contract"])
    integer(m["source_observed_at"], "wallet_source_observation_missing", -62135596800, 253402300799)


def build_sql(m, *, fixture_nonce=None):
    validate_preview(m)
    state = U.state_sql(m["before_state"]["inventory"], m["migration_id"])
    guard = U._guard_sql(m)
    fixture_guard = U._test_fixture_guard(m, fixture_nonce) if fixture_nonce is not None else ""
    inv = m["before_state"]["inventory"]
    seq = deepcopy(m["before_state"]["sequences"])
    counts = {SEQUENCES[0]: 4 * sum(r["initial"] for r in m["positions"]),
              SEQUENCES[1]: sum(r["journal_amount"] != 0 for r in m["positions"])}
    for name, count in counts.items():
        if count:
            seq[name]["last_value"] = C._next_id(m["before_state"], name, count) + count - 1
            seq[name]["is_called"] = True
    body = f"""DECLARE m jsonb := {literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 v_journal_id bigint; after_state jsonb; actual jsonb;
BEGIN
 {fixture_guard}
 {guard}
 LOCK TABLE {','.join(F.relation(n) for n in inv['tables'])} IN ACCESS EXCLUSIVE MODE;
 {guard}
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->>'scope'<>{literal(SCOPE)}
   OR previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->'post_state' IS DISTINCT FROM ({state})
  THEN RAISE EXCEPTION 'wallet_replay_conflict'; END IF;
  {_readback_sql(m)}
 ELSE
  IF EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={literal(SCOPE)}
   AND (manifest->>'source_namespace_sha256'=m->>'source_namespace_sha256' OR EXISTS(
    SELECT 1 FROM jsonb_array_elements(manifest->'material'->'positions') p
    JOIN jsonb_array_elements(m->'positions') q ON p->>'account_id'=q->>'account_id'))) THEN
   RAISE EXCEPTION 'wallet_source_already_reconciled'; END IF;
  IF ({state}) IS DISTINCT FROM m->'before_state' OR ({ledger_facts_sql()}) IS DISTINCT FROM m->'snapshot'->'ledger' THEN
   RAISE EXCEPTION 'wallet_target_state_changed'; END IF;
  {_write_sql(m)}
  SET CONSTRAINTS ALL IMMEDIATE;
  {_readback_sql(m)}
  SELECT ({ledger_facts_sql()}) INTO actual;
  IF actual->'global_valid' IS DISTINCT FROM 'true'::jsonb THEN RAISE EXCEPTION 'wallet_ledger_reconciliation_failed'; END IF;
  IF EXISTS(SELECT 1 FROM jsonb_each({literal(expected_balances(m))}::jsonb) x
   LEFT JOIN core_billing.balance_state b ON b.ledger_account_id=x.key::bigint
   WHERE b.balance_units IS DISTINCT FROM (x.value->>'units')::bigint
    OR b.revision IS DISTINCT FROM (x.value->>'revision')::bigint) THEN
   RAISE EXCEPTION 'wallet_balance_revision_readback_mismatch'; END IF;
  IF ({preservation_sql(m)}) IS DISTINCT FROM m->'before_state'->'tables' THEN
   RAISE EXCEPTION 'wallet_preexisting_rows_changed'; END IF;
  SELECT ({state}) INTO after_state;
  IF after_state->'sequences' IS DISTINCT FROM {literal(seq)}::jsonb THEN
   RAISE EXCEPTION 'wallet_sequence_change_unexpected'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',jsonb_build_object(
   'scope',{literal(SCOPE)},'source_namespace_sha256',m->>'source_namespace_sha256',
   'source_export_sha256',m->>'source_export_sha256','material',m,'post_state',after_state));
 END IF;
 {guard}
END"""
    tag = "$wallet_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "wallet_sql_delimiter_collision")
    return f"""SET standard_conforming_strings=on; SET timezone='UTC'; SET lock_timeout='5s'; SET statement_timeout='180s';
DO $fence$ BEGIN IF NOT pg_try_advisory_lock({U.FENCE}) THEN RAISE EXCEPTION 'wallet_import_busy'; END IF; END $fence$;
BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SET LOCAL row_security=off;
SET LOCAL search_path=pg_catalog,public;
DO {tag}{body}{tag}; COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(m['migration_id'])},'material_sha256',{literal(digest(m))});
"""


def final_fence_observation_sql(namespace):
    """Run only in the retained source maintenance session after its barrier.

    This SQL reads facts; it never stops services, disallows connections,
    terminates sessions, commits prepared transactions or alters source money.
    The operator first records the original datallowconn and service states,
    refuses any prepared transactions, stops the actual writers, then sets the
    source database ALLOW_CONNECTIONS=false and drains other business backends
    while retaining this connected maintenance session. After the barrier,
    execute this query in that same session. Do not reconnect: the barrier
    correctly prevents new connections, including ordinary owner connections.

    If the technical cutover is abandoned BEFORE target financial commit, use
    the retained session (or a separately authorized maintenance database
    session) to restore exactly the original datallowconn, then restart only
    the previously active source writers. After a target commit/UNKNOWN, keep
    the fence and resolve/reconcile first; never automatically reopen the old
    writer and create two writable financial histories. Prepared transactions
    are a hard stop, never automatically committed or rolled back by this tool.
    """
    U.namespace_key(namespace)
    users = F.relation(namespace["schema"] + ".users")
    options = F.relation(namespace["schema"] + ".options")
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL statement_timeout='60s';
SELECT jsonb_build_object(
 'format_version',1,'scope','current-source-wallet-fence-observation-v1',
 'source_namespace',jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
  'database',current_database(),'schema',{literal(namespace['schema'])}),
 'observed_at',to_char(clock_timestamp(),'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
 'local_snapshot',pg_current_snapshot()::text,
 'transaction_read_only',current_setting('transaction_read_only'),
 'database_allows_connections',(SELECT datallowconn FROM pg_database WHERE datname=current_database()),
 'other_database_backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=
  (SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()
  AND backend_type IS DISTINCT FROM 'autovacuum worker'),
 'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()),
 'source_user_count',(SELECT count(*) FROM {users}),
 'invalid_user_rows',(SELECT count(*) FROM {users} WHERE id IS NULL OR quota IS NULL OR aff_quota IS NULL),
 'users',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'quota',quota,'affiliate',aff_quota)
  ORDER BY id),'[]'::jsonb) FROM {users}),
 'unit_options',(SELECT coalesce(jsonb_agg(jsonb_build_object('key',key,'value',value) ORDER BY key),'[]'::jsonb)
  FROM {options} WHERE key IN ('CreditsPerUSD','QuotaPerUnit')));
COMMIT;
"""


def fence_template(m):
    """Unexecutable placeholders only; this is not approval or fence evidence."""
    ref = {"path": "<actual owned 0600 file under existing BACKUP_ROOT>", "sha256": "<actual file SHA256>"}
    return {"template_only": True, "format_version": 1, "scope": "current-source-wallet-fence-v1",
        "source_namespace_sha256": m["source_namespace_sha256"], "source_archive_sha256": m["source_archive_sha256"],
        "source_writes_fenced": None, "verification_actual_exit": None,
        "post_fence_wallet_facts_sha256": None, "post_fence_source_user_count": None,
        "observation": deepcopy(ref), "observation_sql": deepcopy(ref), "verification_stderr": deepcopy(ref),
        "operation_receipt": deepcopy(ref), "operation_receipt_template": {
            "template_only": True, "format_version": 1, "scope": "source-database-write-fence-operations-v1",
            "method": "database-disallow-connections-and-drain", "source_namespace": m["source_namespace"],
            "operator": "<actual operator>", "started_at": "<actual UTC timestamp>", "finished_at": "<actual UTC timestamp>",
            "original_database_allows_connections": None, "original_services": [
                {"unit": "<actual source writer service>", "active_state": "<original state>",
                 "sub_state": "<original state>"}],
            "actions": [{"action": action, "command": "<actual executed command>", "exit_code": None,
                         "stdout": deepcopy(ref), "stderr": deepcopy(ref)} for action in (
                         "stop_source_writers", "disallow_source_database_connections", "drain_source_database_backends")]}}


def utc_time(value, code):
    need(type(value) is str, code)
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise Error(code) from None
    need(result.tzinfo is not None and result.utcoffset().total_seconds() == 0, code)
    return result


def validate_final_fence(m, fence):
    fields = {"format_version", "scope", "source_namespace_sha256", "source_archive_sha256", "source_writes_fenced",
              "verification_actual_exit", "post_fence_wallet_facts_sha256", "post_fence_source_user_count",
              "observation", "observation_sql", "verification_stderr", "operation_receipt"}
    need(type(fence) is dict and set(fence) == fields and fence["format_version"] == 1
         and fence["scope"] == "current-source-wallet-fence-v1"
         and fence["source_namespace_sha256"] == m["source_namespace_sha256"]
         and fence["source_archive_sha256"] == m["source_archive_sha256"]
         and fence["source_writes_fenced"] is True and type(fence["verification_actual_exit"]) is int
         and fence["verification_actual_exit"] == 0, "wallet_final_source_fence_not_verified")
    verify_files({k: fence[k] for k in ("observation", "observation_sql", "verification_stderr", "operation_receipt")})
    need(C.private_bytes(Path(fence["observation_sql"]["path"])) == final_fence_observation_sql(m["source_namespace"]).encode(),
         "wallet_source_fence_query_not_exact")
    observed = C.load_json(Path(fence["observation"]["path"]))
    need(set(observed) == {"format_version", "scope", "source_namespace", "observed_at", "local_snapshot",
         "transaction_read_only", "database_allows_connections", "other_database_backends", "prepared_transactions",
         "source_user_count", "invalid_user_rows", "users", "unit_options"}
         and observed["format_version"] == 1 and observed["scope"] == "current-source-wallet-fence-observation-v1"
         and observed["source_namespace"] == m["source_namespace"]
         and observed["transaction_read_only"] == "on" and observed["database_allows_connections"] is False
         and all(type(observed[k]) is int and observed[k] == 0 for k in (
             "other_database_backends", "prepared_transactions", "invalid_user_rows"))
         and type(observed["local_snapshot"]) is str and observed["local_snapshot"],
         "wallet_source_fence_actual_pg_barrier_missing")
    rows, seen = [], set()
    for row in observed["users"]:
        need(set(row) == {"id", "quota", "affiliate"}, "wallet_fence_row_shape_invalid")
        uid = integer(row["id"], "wallet_fence_user_invalid", 1)
        need(uid not in seen, "wallet_fence_user_duplicate")
        seen.add(uid)
        rows.append({"id": uid, "quota": integer(row["quota"], "wallet_fence_quota_invalid", -SAFE, SAFE),
                     "aff_quota": integer(row["affiliate"], "wallet_fence_affiliate_invalid", -SAFE, SAFE)})
    unit_basis({"options": observed["unit_options"], "archive_sha256": m["source_archive_sha256"],
                "export_sha256": fence["observation"]["sha256"], "source_kind": "actual-post-fence-readonly-query"})
    need(type(observed["source_user_count"]) is int and len(rows) == observed["source_user_count"] == m["source_users"]
         == fence["post_fence_source_user_count"] and json_sha(sorted(rows, key=lambda r: r["id"]))
         == m["source_wallet_facts_sha256"] == fence["post_fence_wallet_facts_sha256"],
         "wallet_final_source_fence_not_verified")
    operation = C.load_json(Path(fence["operation_receipt"]["path"]))
    need(set(operation) == {"format_version", "scope", "method", "source_namespace", "operator", "started_at", "finished_at",
                           "original_database_allows_connections", "original_services", "actions"}
         and operation["format_version"] == 1 and operation["scope"] == "source-database-write-fence-operations-v1"
         and operation["method"] == "database-disallow-connections-and-drain"
         and operation["source_namespace"] == m["source_namespace"] and type(operation["operator"]) is str
         and operation["operator"].strip() and type(operation["original_database_allows_connections"]) is bool,
         "wallet_source_fence_operation_receipt_missing")
    need(utc_time(operation["started_at"], "wallet_fence_time_invalid") <=
         utc_time(operation["finished_at"], "wallet_fence_time_invalid") <=
         utc_time(observed["observed_at"], "wallet_fence_time_invalid"), "wallet_fence_time_order_invalid")
    need(type(operation["original_services"]) is list and operation["original_services"] and all(
         set(s) == {"unit", "active_state", "sub_state"} and all(type(v) is str and v.strip() for v in s.values())
         for s in operation["original_services"]), "wallet_original_source_services_missing")
    actions = operation["actions"]
    need(type(actions) is list and [a["action"] for a in actions] == ["stop_source_writers",
         "disallow_source_database_connections", "drain_source_database_backends"], "wallet_fence_actions_missing")
    for action in actions:
        need(set(action) == {"action", "command", "exit_code", "stdout", "stderr"}
             and type(action["command"]) is str and action["command"].strip()
             and type(action["exit_code"]) is int and action["exit_code"] == 0, "wallet_fence_action_not_successful")
        verify_files({"stdout": action["stdout"], "stderr": action["stderr"]})


def validate_source_review(m, review, review_file):
    need(type(review) is dict, "wallet_source_review_must_be_object")
    expected = {"format_version": 1, "scope": SCOPE, "decision": "review-current-wallet-debt-affiliate-reconciliation",
        "source_namespace_sha256": m["source_namespace_sha256"], "source_archive_sha256": m["source_archive_sha256"],
        "source_export_sha256": m["source_export_sha256"], "identity_map_sha256": m["identity_map_sha256"],
        "positions_sha256": m["mapping_sha256"], "units_sha256": digest(m["units"]),
        "other_rights_policy": OTHER_RIGHTS, "wallet_policy": POLICY,
        "source_review_evidence": review.get("source_review_evidence"),
        "source_write_fence_evidence": review.get("source_write_fence_evidence")}
    need(review == expected, "wallet_actual_source_review_missing_or_mismatched")
    for key in ("source_review_evidence", "source_write_fence_evidence"):
        verify_files({key: review[key]})
        need(Path(review[key]["path"]) != Path(review_file), "wallet_review_cannot_attest_itself")
    fence = C.load_json(Path(review["source_write_fence_evidence"]["path"]))
    validate_final_fence(m, fence)


def validate_approval(m, sql, approval, review_ref):
    # An exact external authorization record, never emitted by prepare().
    need(type(approval) is dict, "wallet_execution_approval_must_be_object")
    record = approval.get("user_authorization_record")
    need(type(record) is dict, "wallet_user_authorization_record_required")
    verify_files({"authorization": record})
    authorization = C.load_json(Path(record["path"]))
    binding = {"format_version": 1, "scope": SCOPE, "decision": DECISION,
        "material_sha256": digest(m), "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(),
        "source_review_sha256": review_ref["sha256"]}
    need(approval == {**binding, "user_authorization_record": record}, "wallet_execution_approval_mismatch")
    need(set(authorization) == set(binding) | {"user_id", "message_reference", "approved_at", "exact_user_message"}
         and all(authorization[k] == v for k, v in binding.items()), "wallet_authorization_binding_mismatch")
    need(all(type(authorization[k]) is str and len(authorization[k].strip()) > 0
             for k in ("user_id", "message_reference", "approved_at", "exact_user_message")),
         "wallet_user_authorization_origin_missing")
    # Obtaining and authenticating that user's original message is the existing
    # operator approval boundary, not a claim derived from hashes or free text.


def resolve_sql(m, original_application):
    validate_preview(m)
    need(re.fullmatch(r"lmm-current-wallet-[0-9a-f]{32}", original_application), "wallet_attempt_identity_invalid")
    body = f"""DECLARE r core_migration.import_runs%ROWTYPE; BEGIN
 IF ({F.instance_sql()}) IS DISTINCT FROM {literal(m['snapshot']['instance'])}::jsonb THEN
  RAISE EXCEPTION 'wallet_resolve_instance_changed'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name={literal(original_application)}
  AND datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()) THEN
  RAISE EXCEPTION 'wallet_original_attempt_still_active'; END IF;
 SELECT * INTO r FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])};
 IF FOUND THEN
  IF r.manifest->'material' IS DISTINCT FROM {literal(m)}::jsonb
   OR r.manifest->>'scope'<>{literal(SCOPE)} OR r.source_archive_sha256<>{literal(m['source_archive_sha256'])}
   OR r.mapping_sha256<>{literal(m['mapping_sha256'])} THEN RAISE EXCEPTION 'wallet_resolve_receipt_conflict'; END IF;
  {_readback_sql(m, current_balances=False)}
  IF ({ledger_facts_sql()})->'global_valid' IS DISTINCT FROM 'true'::jsonb THEN
   RAISE EXCEPTION 'wallet_resolve_ledger_integrity_failed'; END IF;
 END IF;
END"""
    tag = "$wallet_resolve_" + hashlib.sha256(body.encode()).hexdigest()[:20] + "$"
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';
DO {tag}{body}{tag};
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={literal(m['migration_id'])}) THEN 'COMMITTED' ELSE 'UNKNOWN' END,
 'automatic_retry_allowed',false,'material_sha256',{literal(digest(m))}); COMMIT;
"""


def summary(m):
    counts = {}
    for b in m["blockers"]:
        counts[b["code"]] = counts.get(b["code"], 0) + 1
    return {"status": "PREVIEW_REQUIRES_EXPLICIT_APPROVAL" if not counts else "BLOCKED_PREVIEW",
        "source_users": m["source_users"], "mapped_and_reconciled_users": len(m["positions"]),
        "existing_wallet_changes": sum(not r["initial"] and r["source_S"] != r["opening_B"] for r in m["positions"]),
        "affiliate_changes": sum(r["legs"]["affiliate"] != 0 for r in m["positions"]),
        "initial_openings": sum(r["initial"] for r in m["positions"]), "blockers": counts,
        "material_sha256": digest(m), "full_financial_import_complete": False, "production_writes": 0}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("preview")
    for name in ("source-bundle", "identity-map", "target-snapshot", "core78-manifest", "source-namespace", "runtime-roles", "output"):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--migration-id", required=True)
    p.add_argument("--source-observed-at", type=int, help="Optional assertion of the actual source observation epoch")
    for command in ("apply", "resolve"):
        p = sub.add_parser(command)
        for name in ("material", "connection", "output"):
            p.add_argument("--" + name, type=Path, required=True)
        if command == "apply":
            p.add_argument("--source-review", type=Path, required=True)
            p.add_argument("--approval", type=Path, required=True)
        else:
            p.add_argument("--attempt", type=Path, required=True)
    a = parser.parse_args(argv)
    try:
        a.output.mkdir(mode=0o700, parents=True, exist_ok=True)
        if a.command == "preview":
            source, baselines, files = load_sources(a.source_bundle)
            for name in ("identity_map", "target_snapshot", "core78_manifest", "source_namespace", "runtime_roles"):
                files[name] = file_ref(getattr(a, name))
            m = prepare(source, baselines, json.loads(C.private_bytes(a.identity_map)), C.load_json(a.target_snapshot),
                U.core78_contract(C.private_bytes(a.core78_manifest)), C.load_json(a.source_namespace), a.migration_id,
                files, json.loads(C.private_bytes(a.runtime_roles)))
            integer(m["source_observed_at"], "wallet_source_observation_invalid", -62135596800, 253402300799)
            need(a.source_observed_at is None or a.source_observed_at == m["source_observed_at"], "wallet_source_observation_mismatch")
            F.write_private(a.output, "wallet-reconciliation.private.json", canonical(m) + "\n")
            F.write_private(a.output, "final-source-fence.observation.sql", final_fence_observation_sql(m["source_namespace"]))
            F.write_private(a.output, "final-source-fence.template.private.json", canonical(fence_template(m)) + "\n")
            if not m["blockers"]:
                F.write_private(a.output, "wallet-reconciliation.preview.sql", build_sql(m))
            result = summary(m)
            F.write_private(a.output, "wallet-reconciliation.public.json", canonical(result) + "\n")
            print(json.dumps(result))
            return 0 if not m["blockers"] else 2
        m = C.load_json(a.material)
        validate_preview(m)
        connection = C.load_json(a.connection)
        if a.command == "resolve":
            attempt = C.load_json(a.attempt)
            need(attempt["material_sha256"] == digest(m), "wallet_attempt_material_mismatch")
            code, raw = F.run_sql(connection, resolve_sql(m, attempt["application_name"]), a.output,
                "lmm-current-wallet-resolve-" + uuid.uuid4().hex, readonly=True, label="resolve")
            need(code == 0, "wallet_resolution_unconfirmed")
            print(raw.strip())
            return 0
        verify_files(m["source_files"])
        # Rebuild facts from the same immutable source files before approval;
        # file hashes alone are never used as substitutes for the equations.
        source, baselines, _ = load_sources(m["source_files"]["source_bundle"]["path"])
        rebuilt = prepare(source, baselines, json.loads(C.private_bytes(Path(m["source_files"]["identity_map"]["path"]))),
            C.load_json(Path(m["source_files"]["target_snapshot"]["path"])),
            U.core78_contract(C.private_bytes(Path(m["source_files"]["core78_manifest"]["path"]))),
            m["source_namespace"], m["migration_id"], m["source_files"], m["runtime_roles"])
        need(rebuilt == m, "wallet_preview_rebuild_mismatch")
        review = C.load_json(a.source_review)
        validate_source_review(m, review, a.source_review)
        sql = build_sql(m)
        validate_approval(m, sql, C.load_json(a.approval), file_ref(a.source_review))
        verify_files(m["source_files"])
        application = "lmm-current-wallet-" + uuid.uuid4().hex
        F.write_private(a.output, "attempt.private.json", canonical({"application_name": application,
            "material_sha256": digest(m), "status": "MAY_HAVE_BEEN_SENT", "automatic_retry_allowed": False,
            "source_review": review, "source_review_file": file_ref(a.source_review),
            "execution_approval": C.load_json(a.approval), "execution_approval_file": file_ref(a.approval)}) + "\n")
        F.write_private(a.output, "approved-wallet-reconciliation.sql", sql)
        try:
            code, raw = F.run_sql(connection, sql, a.output, application, label="apply")
        except (OSError, TimeoutError, subprocess.TimeoutExpired):
            code, raw = 1, ""
        if code != 0:
            print(json.dumps({"status": "UNKNOWN", "automatic_retry_allowed": False}))
            return 3
        try:
            result = json.loads(raw)
        except ValueError:
            result = None
        if result != {"status": "COMMITTED", "migration_id": m["migration_id"], "material_sha256": digest(m)}:
            print(json.dumps({"status": "UNKNOWN", "automatic_retry_allowed": False}))
            return 3
        print(json.dumps(result))
        return 0
    except (Error, C.Error, I.MappingError, R.F.MappingError, ValueError, KeyError, TypeError, OSError) as exc:
        code = str(exc) if re.fullmatch(r"[A-Za-z0-9_]+", str(exc)) else "wallet_private_input_invalid"
        print(json.dumps({"status": "BLOCKED", "blockers": [code]}))
        return 2


if __name__ == "__main__":
    sys.exit(main())
