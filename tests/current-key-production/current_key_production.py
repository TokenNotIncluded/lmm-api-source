#!/usr/bin/env python3
"""Private current-Core78 key append wrapper with actual reconciliation gates.

The tested used-target key and wallet modules remain byte-for-byte frozen.
This wrapper admits a production transaction only after rebuilding the source
facts, checking committed identity/wallet rows, checking continuing rights,
and reading back the exact target evidence under the owner's table locks.
It does not create a financial approval, a full-opening receipt or a source
fence. Missing product support is reported as a specific unresolved right.
"""
from __future__ import annotations

import argparse
from collections import Counter
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import re
import sys
import uuid

import migrate_current_runtime_keys as U
import migrate_current_wallet_reconciliation as W
import legacy_routing_migration as R
import current_key_lineage as L

C, P, Q, F, I = U.C, U.P, U.Q, U.F, U.I
need, Error, digest, canonical, literal = U.need, U.Error, U.digest, U.canonical, U.literal
SCOPE, BLOCKER, TABLES, SEQUENCES, FENCE = U.SCOPE, U.BLOCKER, U.TABLES, U.SEQUENCES, U.FENCE
HEX, IDENTIFIER = U.HEX, U.IDENTIFIER
PRODUCTION_SCOPE = "current-runtime-key-production-reconciliation-v1"
FROZEN = {
    "migrate_current_runtime_keys.py": "6a612f6c07737bc478bff430804f25ffe85072aba6ecfd3a398e4afe16cf9708",
    "migrate_current_wallet_reconciliation.py": "7b05d231bc08150196bd9fa577c3f53a532b78a5249c8143837498ef7e444e46",
}
RIGHTS_TABLES = (
    "core_billing.legacy_pending_liabilities", "core_billing.legacy_pending_liability_receipts",
    "core_billing.legacy_escrow_batch_runs", "core_billing.legacy_escrow_projects",
    "core_billing.legacy_escrow_claims", "core_billing.legacy_escrow_disputes",
    "core_billing.legacy_escrow_state", "core_billing.legacy_escrow_operations",
    "core_payments.legacy_bases", "core_payments.legacy_applications",
    "core_charging.subscriptions", "core_charging.subscription_opening_consumption",
    "core_charging.requests", "core_promotions.campaigns", "core_promotions.codes",
    "core_promotions.claims", "core_checkins.days", "core_checkins.seed_receipts",
    "core_checkins.active_policy", "core_checkins.policy_revisions",
    "core_identity.user_routing_groups", "public.routing_groups", "public.routing_active_release",
)


def frozen_dependencies():
    L.frozen_identity_module()
    for module in (U, W):
        path = Path(module.__file__)
        need(hashlib.sha256(path.read_bytes()).hexdigest() == FROZEN[path.name],
             "current_keys_frozen_dependency_changed")


def tool_sources():
    frozen_dependencies()
    return {**U.tool_sources(), **FROZEN,
            "migrate_current_identity.py": L.IDENTITY_MODULE_SHA256,
            Path(__file__).name: hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            Path(L.__file__).name: hashlib.sha256(Path(L.__file__).read_bytes()).hexdigest()}


def read_ref(ref, *, parse=True):
    need(type(ref) is dict and set(ref) == {"path", "sha256"}, "current_keys_evidence_ref_invalid")
    F.checked_hash(ref["sha256"])
    raw = C.private_bytes(Path(ref["path"]))
    need(hashlib.sha256(raw).hexdigest() == ref["sha256"], "current_keys_evidence_file_changed")
    return json.loads(raw, object_pairs_hook=P.unique_object) if parse else raw


def file_ref(path):
    path = Path(path).absolute()
    return {"path": str(path), "sha256": hashlib.sha256(C.private_bytes(path)).hexdigest()}


def read_source(evidence):
    """Original all-table verifier, followed by the original key projection."""
    inputs = evidence["source"]
    receipt, proof, runtime = (read_ref(inputs[k]) for k in ("receipt", "proof", "runtime"))
    hashes = {Path(inputs[k]["path"]): inputs[k]["sha256"] for k in ("receipt", "proof", "runtime")}
    tables, actual, _ = R.load_routing_export(receipt, Path(inputs["proof"]["path"]), hashes,
                                           retained_tables=tuple(sorted(proof["selected_table_counts"])))
    need(proof == actual, "current_keys_source_proof_changed")
    _, _, _, _, schemas = R.F.metadata(receipt)
    need(all(R.F.digest(schemas[t]) == R.F.digest(proof["source_column_schema"][t]) for t in tables),
         "current_keys_source_schema_changed")
    R.verify_files(hashes)
    provenance = {"source_receipt_sha256": inputs["receipt"]["sha256"],
        "source_export_proof_sha256": inputs["proof"]["sha256"],
        "legacy_runtime_observation_sha256": inputs["runtime"]["sha256"]}
    source, projection = P.source_projection_from_export(tables, proof, snapshot_at=inputs["snapshot_at"],
                                                       runtime_observation=runtime, provenance=provenance)
    unit = W.unit_basis({"archive_sha256": proof["source_archive_sha256"],
        "export_sha256": proof["source_export_sha256"], "options": tables["options"],
        "source_kind": "canonical-source-export"})
    return tables, proof, source, projection, unit


def full_source_fence_sql(namespace, proof):
    """Read in the retained maintenance connection after the real source barrier.

No ALTER/termination/drain is performed here. Output is private JSONL and
contains original credential fields. Do not print it in a terminal response.
Both boundary records are checked; every selected source row is compared as a
duplicate-preserving typed-row multiset against the canonical archive.
"""
    U.namespace_key(namespace)
    schema = namespace["schema"]
    need(IDENTIFIER.fullmatch(schema), "current_keys_source_schema_invalid")
    header = """SELECT json_build_object('kind','source-fence','row',json_build_object(
 'namespace',json_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
 'database',current_database(),'schema',""" + literal(schema) + """),
 'transaction_read_only',current_setting('transaction_read_only'),
 'database_allows_connections',(SELECT datallowconn FROM pg_database WHERE datname=current_database()),
 'other_database_backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database
  WHERE datname=current_database()) AND pid<>pg_backend_pid() AND backend_type IS DISTINCT FROM 'autovacuum worker'),
 'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()),
 'local_snapshot',pg_current_snapshot()::text,
 'observed_at',to_char(clock_timestamp(),'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')));\n"""
    sql = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\nSET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n"
    sql += "DO $clear$ BEGIN PERFORM pg_stat_clear_snapshot(); END $clear$;\n" + header
    for table in sorted(proof["selected_table_counts"]):
        need(IDENTIFIER.fullmatch(table), "current_keys_source_table_invalid")
        sql += "SELECT json_build_object('kind','row','table'," + literal(table) + ", 'row',row_to_json(t)) FROM " + \
            F.relation(schema + "." + table) + " t ORDER BY row_to_json(t)::text;\n"
    return sql + "DO $clear$ BEGIN PERFORM pg_stat_clear_snapshot(); END $clear$;\n" + header + "COMMIT;\n"


def validate_full_source_fence(evidence, tables, proof, wallet):
    W.validate_final_fence(wallet, read_ref(evidence["wallet_source_fence"]))
    item = evidence["full_source_fence"]
    need(type(item.get("actual_exit")) is int and item["actual_exit"] == 0,
         "current_keys_full_source_fence_failed")
    expected_sql = full_source_fence_sql(evidence["source_namespace"], proof)
    need(read_ref(item["sql"], parse=False) == expected_sql.encode(), "current_keys_full_source_fence_query_changed")
    read_ref(item["stderr"], parse=False)
    path = I.private_backup_path(Path(item["output"]["path"]))
    need(I.sha256_file(path) == item["output"]["sha256"], "current_keys_full_source_fence_file_changed")
    counts = {t: Counter(R.F.digest(r) for r in rows) for t, rows in tables.items()}
    observations = []
    closed = False
    with path.open() as stream:
        for line in stream:
            record = R.F.parse_json(line)
            if record.get("kind") == "source-fence":
                need(not closed and len(observations) < 2,
                     "current_keys_full_source_fence_framing_invalid")
                observations.append(record["row"])
                closed = len(observations) == 2
                continue
            table, row = record.get("table"), record.get("row")
            need(len(observations) == 1 and not closed and record.get("kind") == "row"
                 and table in counts and type(row) is dict,
                 "current_keys_full_source_fence_framing_invalid")
            h = R.F.digest(row)
            need(counts[table][h] > 0, "current_keys_source_changed_since_archive")
            counts[table][h] -= 1
    need(len(observations) == 2 and all(not any(c.values()) for c in counts.values()),
         "current_keys_full_source_fence_incomplete")
    for o in observations:
        need(o["namespace"] == evidence["source_namespace"] and o["transaction_read_only"] == "on"
             and o["database_allows_connections"] is False
             and type(o["other_database_backends"]) is int and o["other_database_backends"] == 0
             and type(o["prepared_transactions"]) is int and o["prepared_transactions"] == 0,
             "current_keys_source_barrier_not_held")
    need(observations[0]["local_snapshot"] == observations[1]["local_snapshot"]
         and W.utc_time(observations[0]["observed_at"], "current_keys_source_fence_time_invalid") <=
         W.utc_time(observations[1]["observed_at"], "current_keys_source_fence_time_invalid"),
         "current_keys_source_fence_snapshot_changed")
    need(I.sha256_file(path) == item["output"]["sha256"], "current_keys_full_source_fence_file_changed")
    return observations[-1]


def native_evidence_sql(mapping, exclude_migration_id=None):
    where = " WHERE r.migration_id<>" + literal(exclude_migration_id) if exclude_migration_id is not None else ""
    fields = ["'instance',(" + F.instance_sql() + ")", "'identity_authority',(" + L.identity_authority_sql(mapping) + ")",
        "'ledger',(" + W.ledger_facts_sql() + ")",
        "'imports',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.migration_id),'[]'::jsonb) FROM core_migration.import_runs r" + where + ")",
        "'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database())"]
    for table in RIGHTS_TABLES:
        fields.append(literal(table) + ",(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]'::jsonb) FROM " +
                      F.relation(table) + " r)")
    return "SELECT jsonb_build_object(" + ",".join(fields) + ")"


def native_observation_sql(mapping):
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\nSET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n" + \
        native_evidence_sql(mapping) + ";\nCOMMIT;\n"


def validate_wallet(source_tables, source, mapping, namespace, wallet, native):
    W.validate_preview(wallet)
    W.verify_files(wallet["source_files"])
    current, historical, _ = W.load_sources(wallet["source_files"]["source_bundle"]["path"])
    rebuilt = W.prepare(current, historical, mapping, wallet["snapshot"], wallet["contract"], namespace,
                        wallet["migration_id"], wallet["source_files"], wallet["runtime_roles"])
    need(rebuilt == wallet, "current_keys_wallet_equations_not_rebuilt")
    need(wallet["source_archive_sha256"] == source["source_archive_sha256"]
         and wallet["source_namespace"] == namespace and wallet["identity_map"] == mapping
         and wallet["source_users"] == len(source_tables["users"]), "current_keys_wallet_source_or_map_mismatch")
    rows = {r["id"]: r for r in source_tables["users"]}
    for p in wallet["positions"]:
        need(p["source_S"] == rows[p["source_user_id"]]["quota"]
             and p["source_affiliate"] == rows[p["source_user_id"]]["aff_quota"],
             "current_keys_wallet_current_source_amount_changed")
    runs = [r for r in native["imports"] if r["migration_id"] == wallet["migration_id"]]
    need(len(runs) == 1 and runs[0]["manifest"]["scope"] == W.SCOPE
         and runs[0]["manifest"]["material"] == wallet
         and runs[0]["source_archive_sha256"] == wallet["source_archive_sha256"]
         and runs[0]["mapping_sha256"] == wallet["mapping_sha256"],
         "current_keys_committed_current_wallet_required")
    ledger = native["ledger"]
    need(ledger["global_valid"] is True, "current_keys_native_ledger_invalid")
    owners = {r["account_id"]: r for r in ledger["owners"]}
    for p in wallet["positions"]:
        owner = owners[p["account_id"]]
        buckets = {b["bucket"]: b for b in owner["buckets"]}
        need(owner["user_id"] == p["target_user_id"] and set(buckets) == set(W.BUCKETS),
             "current_keys_native_wallet_owner_invalid")
        for k, value in p["target_buckets"].items():
            W.validate_path(buckets[k])
            need(buckets[k]["units"] == value, "current_keys_wallet_changed_after_reconciliation")
    return {"wallet_material_sha256": digest(wallet), "wallet_migration_id": wallet["migration_id"],
            "wallet_owner_count": len(wallet["positions"]), "ledger_valid": True}


def rights_facts(tables, native, mapping, snapshot_at):
    """Explicit current states only. Unknown/new source states reject.

An immutable pending base's original unknown value is never called the current
settlement state. Actual appended receipts and payment applications are read
separately and retained in the reviewed evidence.
"""
    blockers, facts = [], {}
    owners = {m["source_user_id"]: m for m in mapping}
    for table, field, terminal in (
        ("wallet_transfers", "status", {"claimed", "cancelled"}),
        ("subscription_pre_consume_records", "status", {"settled", "refunded"}),
        ("merchant_store_refunds", "status", {"completed"}),
        ("tool_market_calls", "settlement_status", {"settled", "released"}),
        ("discount_code_reservations", "status", {"consumed", "released"}),
        ("subscription_reset_vouchers", "status", {"redeemed"}),
    ):
        need(table in tables, "current_keys_rights_source_table_missing")
        unknown = [r for r in tables[table] if r.get(field) not in terminal]
        facts[table] = {"rows": len(tables[table]), "source_rows_sha256": R.F.digest(tables[table]), "unresolved": len(unknown)}
        if unknown:
            blockers.append("current_rights_unresolved_" + table)
    if any(r.get("status") not in {"paid", "refunded"} or r.get("fee_held") is not False
           for r in tables["merchant_store_orders"]):
        blockers.append("current_rights_merchant_fee_or_order_unresolved")
    if any(r.get("status") != "expired" or type(r.get("end_time")) is not int or r["end_time"] > snapshot_at
           for r in tables["user_subscriptions"]):
        blockers.append("current_rights_active_subscription_requires_contract")
    source_pending = [("legacy.top_ups", r) for r in tables["top_ups"] if R.F.PAYMENT_LIFECYCLE.topup_can_settle(r)]
    source_pending += [("legacy.subscription_orders", r) for r in tables["subscription_orders"] if r.get("status") == "pending"]
    pending = native["core_billing.legacy_pending_liabilities"]
    covered = []
    for table, row in source_pending:
        owner = owners.get(row["user_id"])
        if owner is None:
            blockers.append("current_rights_orphan_payment_owner_unresolved")
            continue
        matches = [p for p in pending if p["source_table"] == table and p["source_row_key"] == str(row["id"])]
        if len(matches) != 1:
            blockers.append("current_rights_pending_native_basis_missing_or_ambiguous")
            continue
        p = matches[0]
        bound = p["source_row_sha256"] == R.F.digest(row)
        if table == "legacy.subscription_orders":
            payload = p["payload_json"]
            raw = payload.get("source_json_text")
            bound = (type(raw) is str and payload.get("source_row_hash_domain") == "postgres-jsonb-text-utf8-sha256"
                     and hashlib.sha256(raw.encode()).hexdigest() == p["source_row_sha256"]
                     and R.F.digest(R.F.parse_json(raw)) == R.F.digest(row))
        if not (bound and p["owner_user_id"] == owner["target_user_id"]
                and p["owner_account_id"] == owner["target_personal_account_id"]
                and p["spendable"] is False and p["spendable_units"] == 0):
            blockers.append("current_rights_pending_basis_changed")
            continue
        receipts = [r for r in native["core_billing.legacy_pending_liability_receipts"] if all(
            r[k] == p[k] for k in ("migration_id", "source_table", "source_row_key"))]
        bases = [r for r in native["core_payments.legacy_bases"] if table == "legacy.top_ups"
                 and r["source_order_id"] == row["id"] and r["source_user_id"] == row["user_id"]]
        applications = [a for a in native["core_payments.legacy_applications"] if a["basis_id"] in {b["id"] for b in bases}]
        covered.append({"source_table": table, "source_id": row["id"], "source_row_sha256": R.F.digest(row),
            "immutable_original_settlement_state": p["settlement_state"], "actual_receipts": receipts,
            "actual_payment_applications": applications, "current_state_inferred_from_original_base": False})
    facts["pending"] = {"source_rows": len(source_pending), "covered": covered}
    escrow_projects = native["core_billing.legacy_escrow_projects"]
    for row in tables["open_source_bounty_projects"]:
        value = row.get("escrow_quota")
        need(type(value) is int and value >= 0, "current_rights_unknown_escrow_amount")
        if not value:
            continue
        matches = [p for p in escrow_projects if p["source_project_id"] == row["id"]
                   and p["source_row_sha256"] == R.F.digest(row)]
        if len(matches) != 1 or matches[0]["escrow_units"] != value:
            blockers.append("current_rights_escrow_source_or_amount_changed")
            continue
        p = matches[0]
        states = [s for s in native["core_billing.legacy_escrow_state"] if all(s[k] == p[k] for k in ("migration_id", "source_project_id"))]
        operations = [o for o in native["core_billing.legacy_escrow_operations"] if all(o[k] == p[k] for k in ("migration_id", "source_project_id"))]
        # Later legitimate escrow spending is never silently overwritten or
        # mistaken for a source loss. It needs its own explicit delta review.
        if len(states) != 1 or states[0]["remaining_units"] != value or states[0]["closed"] or operations:
            blockers.append("current_rights_escrow_native_activity_requires_review")
    campaigns = {r["id"]: r for r in native["core_promotions.campaigns"]}
    codes = {r["digest"]: r for r in native["core_promotions.codes"]}
    claim_digests = {r["code_digest"] for r in native["core_promotions.claims"] if r["code_digest"] is not None}
    redeemable = []
    required_campaigns = Counter()
    plans = {r["id"] for r in tables["subscription_plans"]}
    for row in tables["redemptions"]:
        if not (row["status"] == 1 and row["deleted_at"] is None and (not row["expired_time"] or row["expired_time"] >= snapshot_at)):
            continue
        reward_type = row["reward_type"].strip().lower() or "quota"
        if reward_type == "reset_voucher":
            # The legacy code has a second, strict voucher expiry check. An
            # unexpired bearer code alone does not establish a usable reward.
            if row["reset_plan_id"] in plans and row["reset_plan_id"] > 0 and (
                    not row["reset_voucher_expires_at"] or row["reset_voucher_expires_at"] > snapshot_at):
                blockers.append("current_rights_reset_voucher_claim_contract_missing")
            continue
        if reward_type != "quota":
            blockers.append("current_rights_nonquota_redemption_requires_contract")
            continue
        key = "\\x" + hashlib.sha256(row["key"].strip().encode()).hexdigest()
        c = codes.get(key)
        campaign = campaigns.get(c["campaign_id"]) if c else None
        if not (c and not c["revoked"] and key not in claim_digests and campaign
                and campaign["kind"] == "redemption" and campaign["award_units"] == row["quota"]
                and campaign["enabled"] and campaign["starts_at"] <= snapshot_at < campaign["ends_at"]
                and campaign["ends_at"] >= (row["expired_time"] + 1 if row["expired_time"] else 253402300799)
                and campaign["reserved_units"] >= row["quota"] and campaign["reserved_claims"] > 0):
            blockers.append("current_rights_redeemable_code_not_preserved")
        if campaign is not None:
            required_campaigns[campaign["id"]] += 1
        redeemable.append({"source_id": row["id"], "code_sha256": key[2:], "quota": row["quota"]})
    if any(campaigns[c]["reserved_claims"] < n or
           campaigns[c]["reserved_units"] < n * campaigns[c]["award_units"]
           for c, n in required_campaigns.items()):
        blockers.append("current_rights_redemption_reservations_insufficient")
    facts["redeemable_codes"] = redeemable
    offered = [r for r in tables["assistant_new_user_gifts"] if r["status"] == "offered"]
    if offered:
        blockers.append("current_rights_offered_gift_claim_contract_missing")
    facts["offered_gifts"] = [{"source_id": r["id"], "source_row_sha256": R.F.digest(r)} for r in offered]
    if tables["checkins"]:
        # The currently verified Go claim path checks core-checkins journal
        # operations, not core_checkins.days. A historical row alone cannot
        # establish deduplication. Connect the real addon after its review.
        blockers.append("checkin_runtime_history_deduplication_unproven")
    facts["checkins"] = {"rows": len(tables["checkins"]), "source_rows_sha256": R.F.digest(tables["checkins"]),
                         "legacy_calendar_offset_seconds": 28800}
    facts["blockers"] = sorted(set(blockers))
    return facts


def validate_reservations(evidence, tables, proof, wallet, source_end):
    """Reuse the existing all-owner zero-nonescrow validator, not a new seal."""
    inflight = read_ref(evidence["inflight"])
    process = read_ref(evidence["source_process_drain"])
    context = {"inflight_evidence": inflight, "source_archive_sha256": proof["source_archive_sha256"],
        "source_export_sha256": proof["source_export_sha256"], "mapping_sha256": wallet["mapping_sha256"],
        "identity_map_sha256": wallet["identity_map_sha256"],
        "source_files": {"source_export": {"sha256": proof["source_export_sha256"]},
                         "inflight_process_evidence": evidence["source_process_drain"]},
        "escrow_unit_evidence": {"source_application_revision": P.LEGACY_REVISION},
        "escrow_batch": {"observed_at": inflight.get("observed_at")},
        "counters": [{"user_id": r["id"]} for r in tables["users"]],
        "source_plan": {"source_evidence": {t: {"row_count": len(rows)} for t, rows in tables.items()}}}
    F.validate_inflight(context)
    need(process.get("format") == "lmm.source-process-drain-observation.v1"
         and process.get("source_namespace") == evidence["source_namespace"]
         and process.get("source_process_identity") == inflight["source_process_identity"]
         and process.get("drain_completed_at") == inflight["drain_completed_at"]
         and type(process.get("active_requests_after_drain")) is int and process["active_requests_after_drain"] == 0
         and type(process.get("unconfirmed_precharges")) is int and process["unconfirmed_precharges"] == 0,
         "current_keys_actual_application_drain_unproven")
    # These are original process/command observations supplied by the cutover
    # operator. Reading source status counts alone never manufactures them.
    for key in ("drain_command", "drain_stdout", "drain_stderr", "process_identity_observation"):
        read_ref(process[key], parse=False)
    command = read_ref(process["drain_command"])
    need(type(command.get("actual_exit")) is int and command["actual_exit"] == 0
         and type(command.get("argv")) is list and command["argv"],
         "current_keys_source_drain_command_not_successful")
    at = int(W.utc_time(source_end["observed_at"], "current_keys_source_fence_time_invalid").timestamp())
    need(inflight["drain_completed_at"] <= inflight["observed_at"] <= at,
         "current_keys_drain_not_before_final_source_observation")
    return {"inflight_sha256": digest(inflight), "all_owner_reserved_units": 0,
            "covered_owners": len(inflight["positions"]), "actual_source_fence_observed_at": source_end["observed_at"]}


def backup_verification_sql(snapshot, contract, target, migration_id):
    return U.snapshot_sql(contract, snapshot["state"]["inventory"], migration_id,
                          target.get(C.LINEAGE.KEY, []))


def validate_backup(evidence, snapshot, contract, target):
    backup = read_ref(evidence["backup_restore"])
    need(backup.get("format") == "lmm.current-native-backup-restore-observation.v1",
         "current_keys_backup_restore_contract_missing")
    path = I.private_backup_path(Path(backup["archive"]["path"]))
    need(I.sha256_file(path) == backup["archive"]["sha256"], "current_keys_backup_archive_changed")
    source, restored = (read_ref(backup[k]) for k in ("source_observation", "restored_observation"))
    exact_sql = backup_verification_sql(snapshot, contract, target, evidence["key_migration_id"])
    need(read_ref(backup["verification_sql"], parse=False) == exact_sql.encode(),
         "current_keys_restore_verification_query_changed")
    for key in ("source_observation", "restored_observation"):
        need(type(backup.get(key + "_actual_exit")) is int and backup[key + "_actual_exit"] == 0,
             "current_keys_restore_observation_query_failed")
        read_ref(backup[key + "_stderr"], parse=False)
    U.validate_snapshot(source, contract)
    U.validate_snapshot(restored, contract)
    for key, program in (("dump_command", "pg_dump"), ("restore_command", "pg_restore")):
        command = read_ref(backup[key])
        need(type(command.get("actual_exit")) is int and command["actual_exit"] == 0
             and type(command.get("argv")) is list and command["argv"]
             and Path(command["argv"][0]).name == program, "current_keys_backup_restore_command_failed")
        read_ref(command["stdout"], parse=False)
        read_ref(command["stderr"], parse=False)
    need(source == snapshot
         and restored["state"] == snapshot["state"]
         and source["catalog_fingerprint_sha256"] == restored["catalog_fingerprint_sha256"] == U.CORE78_CATALOG_SHA256,
         "current_keys_backup_restore_state_not_current")
    need(restored["instance"]["system_identifier"] != source["instance"]["system_identifier"]
         and restored["instance"]["server_instance_nonce"] != source["instance"]["server_instance_nonce"],
         "current_keys_restore_not_isolated")
    need(type(backup.get("restore_verification_actual_exit")) is int and backup["restore_verification_actual_exit"] == 0,
         "current_keys_restore_verification_failed")
    return {"archive_sha256": backup["archive"]["sha256"], "current_state_sha256": digest(snapshot["state"])}


def inspect_reconciliation(evidence, *, require_ready=True):
    frozen_dependencies()
    need(evidence.get("format_version") == 1 and evidence.get("scope") == PRODUCTION_SCOPE,
         "current_keys_reconciliation_contract_invalid")
    need(type(evidence.get("key_migration_id")) is str and I.MIGRATION_ID.fullmatch(evidence["key_migration_id"]),
         "current_keys_reconciliation_operation_id_missing")
    tables, proof, source, projection, unit = read_source(evidence)
    target = read_ref(evidence["target_projection"])
    snapshot = read_ref(evidence["target_snapshot"])
    contract = U.core78_contract(read_ref(evidence["core78_manifest"], parse=False))
    U.validate_snapshot(snapshot, contract)
    need(snapshot["native_users"] == target["users"], "current_keys_target_projection_stale")
    U.validate_activation_bindings(target, snapshot)
    mapping = target["source_target_id_map"]
    P.target_mapping(target, source["source_archive_sha256"])
    need({r["source_user_id"] for r in mapping} == {r["id"] for r in tables["users"]},
         "current_keys_complete_owner_mapping_missing")
    observed = read_ref(evidence["native_observation"])
    need(type(evidence.get("native_observation_actual_exit")) is int and evidence["native_observation_actual_exit"] == 0
         and read_ref(evidence["native_observation_sql"], parse=False) == native_observation_sql(mapping).encode()
         and observed["instance"] == snapshot["instance"]
         and type(observed["prepared_transactions"]) is int and observed["prepared_transactions"] == 0,
         "current_keys_actual_native_observation_invalid")
    read_ref(evidence["native_observation_stderr"], parse=False)
    identity = [r for r in observed["imports"] if r["migration_id"] == evidence["identity_migration_id"]]
    need(len(identity) == 1, "current_keys_current_identity_receipt_missing")
    bindings = L.validate(tables["users"], source, target, evidence["source_namespace"], identity[0], observed["identity_authority"])
    dormant = L.dormant_bindings(tables, source, target, identity[0], observed)
    im = identity[0]["manifest"]["material"]
    need(im.get("source_canonical_export_proof_sha256") == evidence["source"]["proof"]["sha256"]
         and im.get("source_canonical_receipt_sha256") == evidence["source"]["receipt"]["sha256"],
         "current_keys_identity_canonical_files_mismatch")
    wallet = read_ref(evidence["wallet_material"])
    wallet_facts = validate_wallet(tables, source, mapping, evidence["source_namespace"], wallet, observed)
    source_end = validate_full_source_fence(evidence, tables, proof, wallet)
    reservations = validate_reservations(evidence, tables, proof, wallet, source_end)
    rights_at = int(W.utc_time(source_end["observed_at"], "current_keys_source_fence_time_invalid").timestamp())
    rights = rights_facts(tables, observed, mapping, rights_at)
    backup = validate_backup(evidence, snapshot, contract, target)
    facts = {"format_version": 1, "scope": PRODUCTION_SCOPE,
        "source_archive_sha256": source["source_archive_sha256"], "source_export_sha256": source["source_export_sha256"],
        "source_projection_sha256": source["source_projection_sha256"],
        "source_namespace_sha256": U.namespace_key(evidence["source_namespace"]),
        "source_table_counts": proof["selected_table_counts"],
        "identity_map_sha256": digest(mapping), "identity_run_sha256": digest(identity[0]),
        "current_identity_bindings_sha256": digest(bindings), "unit": unit,
        "current_dormant_group_bindings_sha256": digest(dormant),
        "historical_units": wallet["units"], "wallet": wallet_facts, "rights": rights,
        "reservations": reservations, "backup_restore": backup,
        "native_observation_sha256": evidence["native_observation"]["sha256"],
        "source_fence_sha256": evidence["full_source_fence"]["output"]["sha256"],
        "target_state_sha256": digest(snapshot["state"]), "blockers": rights["blockers"]}
    if require_ready:
        need(not facts["blockers"], "current_keys_continuing_rights_unresolved")
    return {"source": source, "source_projection": projection, "target": target, "snapshot": snapshot,
            "contract": contract, "wallet": wallet, "native": observed, "bindings": bindings, "dormant": dormant, "facts": facts}


def validate_economic_review(evidence, facts):
    review = read_ref(evidence["economic_review"])
    need(set(review) == {"format_version", "scope", "decision", "facts", "review_authorization"}
         and review["format_version"] == 1 and review["scope"] == PRODUCTION_SCOPE
         and review["decision"] == "accept-current-source-and-native-economic-reconciliation"
         and review["facts"] == facts and not facts["blockers"],
         "current_keys_actual_economic_review_missing")
    record = read_ref(review["review_authorization"])
    binding = {k: review[k] for k in ("format_version", "scope", "decision", "facts")}
    need(record.get("review_body_sha256") == digest(binding)
         and all(type(record.get(k)) is str and record[k].strip() for k in (
             "user_id", "message_reference", "approved_at", "exact_user_message")),
         "current_keys_economic_review_origin_missing")
    # Origin authentication is the existing operator/user boundary. This code
    # verifies the exact facts and reference; it never invents that message.
    return review


def require_current_reconciliation(material, evidence):
    context = inspect_reconciliation(evidence)
    economic = validate_economic_review(evidence, context["facts"])
    need(material.get("production_closure") == {"scope": PRODUCTION_SCOPE,
         "evidence_sha256": digest(evidence), "facts": context["facts"]}
         and material.get("current_identity_bindings") == context["bindings"]
         and material.get("current_dormant_group_bindings") == context["dormant"]
         and material["source_input_sha256"] == P.sha256(context["source"])
         and material["target_projection"] == context["target"]
         and material["snapshot"] == context["snapshot"], "current_keys_production_binding_changed")
    review = material["source_review"]
    need(review["economic_review_sha256"] == digest(economic)
         and review["evidence_sha256"] == evidence["economic_review"]["sha256"],
         "current_keys_credential_review_not_current_economic_review")
    C.validate_review(review, context["source"], context["target"], material["proposal"])
    return context


# Mechanical fork of the pinned used-target adapter: same allocator,
# SQL policy, preserving readback and unknown-commit resolver. The only changes
# are the explicit current lineage mapper and the production pre-allocation guard.
validate_snapshot, validate_activation_bindings = U.validate_snapshot, U.validate_activation_bindings
namespace_key, check_source_identity_conflicts = U.namespace_key, U.check_source_identity_conflicts
state_sql, _guard_sql, _insert_sql, _readback_sql = U.state_sql, U._guard_sql, U._insert_sql, U._readback_sql
_test_fixture_guard = U._test_fixture_guard


def prepare_mechanics(source, target, review, *, snapshot, contract, migration_id,
                      source_namespace, runtime_roles, current_bindings, dormant_bindings, encryption_key=None):
    """Build an auditable private draft, not a production authorization.

    Caller must obtain source/target through the existing export-provenance
    adapter. The production CLI below does so. This pure function also serves
    generated synthetic fixtures and never touches a database.
    """
    validate_snapshot(snapshot, contract)
    need(type(migration_id) is str and I.MIGRATION_ID.fullmatch(migration_id),
         "current_keys_migration_id_invalid")
    need(type(runtime_roles) is list and runtime_roles and runtime_roles == sorted(set(runtime_roles))
         and all(type(r) is str and IDENTIFIER.fullmatch(r) for r in runtime_roles),
         "current_keys_runtime_roles_invalid")
    need(snapshot["instance"]["role"] not in runtime_roles
         and snapshot["instance"]["session_user"] not in runtime_roles, "current_keys_runtime_roles_invalid")
    namespace_key(source_namespace)
    revoke = C._revoke_unsupported(review)
    C.validate_wire_source(source, allow_revoked_unsupported=revoke)
    proposal = L.plan(source, target, current_bindings, dormant_bindings)
    C.validate_review(review, source, target, proposal)
    need(snapshot["native_users"] == target["users"], "current_keys_target_projection_stale")
    validate_activation_bindings(target, snapshot)
    state = snapshot["state"]
    count = len(proposal["records"])
    first = C._next_id(state, SEQUENCES[0], count)
    first_budget = C._next_id(state, SEQUENCES[1], count)
    need(first > snapshot["max_ids"]["credentials"] and first_budget > snapshot["max_ids"]["budgets"],
         "current_keys_sequence_behind_existing_rows")
    rows = L.supported_rows(proposal, target, first, bindings=current_bindings, dormant=dormant_bindings, revoke_unsupported=revoke)
    check_source_identity_conflicts(source_namespace, rows, snapshot["key_imports"])
    raw = {r["id"]: r for r in source["tokens"]}
    for row in rows:
        token = raw[row["source_token_id"]]
        row["source_row_sha256"] = digest(token)
        row["source_deleted_at"] = token["deleted_at"]
        if not row["one_time_reveal"]:
            need(encryption_key is not None, "current_keys_encryption_key_required")
            row["encrypted_secret_hex"] = C.encrypt_secret(encryption_key, row["target_credential_id"],
                                                          row["target_user_id"], token["key"])
            C.verify_encrypted_secret(encryption_key, row, token["key"])
    receipt = C.mapping_receipt(source["source_archive_sha256"], rows)
    user_map = {r["source_user_id"]: {"user_id": r["target_user_id"],
                "account_id": r["target_personal_account_id"]} for r in target["source_target_id_map"]}
    quota = Q.build_plan_from_tokens(source["tokens"], user_map, receipt, migration_id=migration_id,
                                    source_archive_sha256=source["source_archive_sha256"])
    material = {"format_version": 1, "scope": SCOPE, "production_authorized": False,
                "production_blocker": BLOCKER, "migration_id": migration_id,
                "source_archive_sha256": source["source_archive_sha256"],
                "source_namespace": source_namespace, "source_namespace_sha256": namespace_key(source_namespace),
                "source_input_sha256": P.sha256(source), "source_review": review,
                "target_projection": target, "proposal": proposal, "records": rows,
                "credential_receipt": receipt, "quota_plan": quota, "first_budget_id": first_budget,
                "snapshot": snapshot, "before_state": state, "contract": contract,
                "runtime_roles": runtime_roles, "mapping_sha256": digest(receipt["records"]),
                "tool_source_hashes": tool_sources(), "current_identity_bindings": current_bindings,
                "current_dormant_group_bindings": dormant_bindings}
    validate_mechanics(material)
    need(not any(r["key"] in canonical(material) for r in source["tokens"]), "current_keys_secret_echo")
    return material


def validate_mechanics(m):
    need(type(m) is dict and m.get("format_version") == 1 and m.get("scope") == SCOPE
         and m.get("production_authorized") is False and m.get("production_blocker") == BLOCKER,
         "current_keys_private_draft_required")
    validate_snapshot(m["snapshot"], m["contract"])
    need(m.get("tool_source_hashes") == tool_sources(), "current_keys_tool_source_changed")
    validate_activation_bindings(m["target_projection"], m["snapshot"])
    need(m["before_state"] == m["snapshot"]["state"], "current_keys_state_binding_mismatch")
    need(m["source_namespace_sha256"] == namespace_key(m["source_namespace"]),
         "current_keys_namespace_binding_mismatch")
    need(I.MIGRATION_ID.fullmatch(m["migration_id"]), "current_keys_migration_id_invalid")
    need(m["source_archive_sha256"] == m["proposal"]["source_archive_sha256"]
         and m["source_input_sha256"] == m["proposal"]["source_input_sha256"]
         and m["target_projection"]["users"] == m["snapshot"]["native_users"],
         "current_keys_source_target_binding_mismatch")
    rows = m["records"]
    need(type(rows) is list and 1 <= len(rows) <= C.MAX_ALIAS_KEYS, "current_keys_count_invalid")
    first = C._next_id(m["before_state"], SEQUENCES[0], len(rows))
    budget = C._next_id(m["before_state"], SEQUENCES[1], len(rows))
    need(m["first_budget_id"] == budget and first > m["snapshot"]["max_ids"]["credentials"]
         and budget > m["snapshot"]["max_ids"]["budgets"], "current_keys_sequence_binding_invalid")
    expected = L.supported_rows(m["proposal"], m["target_projection"], first,
                                 bindings=m["current_identity_bindings"], dormant=m["current_dormant_group_bindings"],
                                 revoke_unsupported=C._revoke_unsupported(m["source_review"]))
    fields = {"encrypted_secret_hex", "source_row_sha256", "source_deleted_at"}
    need(len(rows) == len(expected) and all({k: v for k, v in a.items() if k not in fields} ==
         {k: v for k, v in b.items() if k not in fields} for a, b in zip(rows, expected)),
         "current_keys_mapper_result_modified")
    for row in rows:
        F.checked_hash(row["source_row_sha256"])
        need((P.deleted(row.get("source_deleted_at")) or bool(row.get("revoked_rekey_required"))) == row["revoked"],
             "current_keys_revocation_binding_invalid")
        encrypted = row["encrypted_secret_hex"]
        need((row["one_time_reveal"] and encrypted is None) or (not row["one_time_reveal"]
             and type(encrypted) is str and re.fullmatch(r"[0-9a-f]{88,600}", encrypted)
             and len(encrypted) % 2 == 0), "current_keys_ciphertext_invalid")
    receipt = C.mapping_receipt(m["source_archive_sha256"], rows)
    need(m["credential_receipt"] == receipt and m["mapping_sha256"] == digest(receipt["records"]),
         "current_keys_mapping_receipt_modified")
    Q.validate_key_quota_plan(m["quota_plan"])
    need(m["quota_plan"]["migration_id"] == m["migration_id"]
         and m["quota_plan"]["source_archive_sha256"] == m["source_archive_sha256"],
         "current_keys_quota_batch_binding_invalid")
    expected_quota = [{"source_key_id": r["source_token_id"], "target_credential_id": r["target_credential_id"],
        "target_user_id": r["target_user_id"], "target_account_id": r["target_personal_account_id"],
        "used_units": r["quota"]["opening_consumed"], "remaining_units": r["quota"]["remaining"],
        "unlimited": r["quota"]["unlimited"], "source_secret_sha256": r["digest_sha256"],
        "source_status": r["source_status"], "source_row_sha256": r["source_row_sha256"]} for r in rows]
    need(m["quota_plan"]["key_records"] == expected_quota, "current_keys_quota_mapping_modified")


def _mechanics_transaction_sql(m, fixture_guard, production_guard=""):
    validate_mechanics(m)
    inv = m["before_state"]["inventory"]
    state = state_sql(inv, m["migration_id"])
    preserved = state_sql(inv, m["migration_id"], inserted=m)
    locks = ",".join(F.relation(n) for n in inv["tables"])
    mapped_ids = ",".join(str(r["source_token_id"]) for r in m["records"])
    expected_sequences = deepcopy(m["before_state"]["sequences"])
    for sequence, first in zip(SEQUENCES, (m["records"][0]["target_credential_id"], m["first_budget_id"])):
        expected_sequences[sequence]["last_value"] = first + len(m["records"]) - 1
        expected_sequences[sequence]["is_called"] = True
    body = f"""
DECLARE m jsonb := {literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; post_state jsonb; preserved_state jsonb;
BEGIN
 {fixture_guard}
 {_guard_sql(m)}
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {_guard_sql(m)}
 {production_guard}
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256'
   OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->>'scope'<>{literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->'mapping' IS DISTINCT FROM m->'credential_receipt'->'records'
   OR previous.manifest->>'source_namespace_sha256' IS DISTINCT FROM m->>'source_namespace_sha256'
  THEN RAISE EXCEPTION 'current_keys_replay_conflict'; END IF;
  {_readback_sql(m)}
  IF previous.manifest->'post_state' IS DISTINCT FROM ({state})
  THEN RAISE EXCEPTION 'current_keys_replay_state_changed'; END IF;
 ELSE
  SELECT ({state}) INTO before_state;
  IF before_state IS DISTINCT FROM m->'before_state' THEN
   RAISE EXCEPTION 'current_keys_target_state_changed'; END IF;
  IF ({C.native_users_sql()}) IS DISTINCT FROM m->'target_projection'->'users'
   OR ({C.LINEAGE.evidence_array_sql(m['target_projection'].get(C.LINEAGE.KEY, []))})
    IS DISTINCT FROM {literal(m['snapshot'].get('admin_activation_evidence', []))}::jsonb THEN
   RAISE EXCEPTION 'current_keys_native_authority_changed'; END IF;
  IF EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={literal(C.SCOPE)}) THEN
   RAISE EXCEPTION 'current_keys_prior_legacy_import_requires_namespace_reconciliation'; END IF;
  IF EXISTS(SELECT 1 FROM core_migration.import_runs r,
   LATERAL jsonb_array_elements(CASE WHEN r.manifest->>'scope'={literal(SCOPE)}
    THEN r.manifest->'mapping' ELSE '[]'::jsonb END) x
   WHERE r.manifest->>'scope'={literal(SCOPE)}
   AND r.manifest->>'source_namespace_sha256'=m->>'source_namespace_sha256'
   AND (x->>'source_key_id')::bigint IN ({mapped_ids})) THEN
   RAISE EXCEPTION 'current_keys_source_identity_already_imported'; END IF;
  IF EXISTS(SELECT 1 FROM core_identity.credentials c JOIN jsonb_array_elements(m->'records') r
   ON c.digest=decode(r->>'digest_sha256','hex')) THEN
   RAISE EXCEPTION 'current_keys_existing_secret_collision'; END IF;
  IF (SELECT coalesce(max(id),0) FROM core_identity.credentials)>={m['records'][0]['target_credential_id']}
   OR (SELECT coalesce(max(id),0) FROM core_charging.budgets)>={m['first_budget_id']} THEN
   RAISE EXCEPTION 'current_keys_sequence_behind_existing_rows'; END IF;
  -- All source/native CAS, namespace, collision, role, catalog and fixture
  -- guards above run before the first non-transactional sequence allocation.
  {_insert_sql(m)}
  SET CONSTRAINTS ALL IMMEDIATE;
  {_readback_sql(m)}
  SELECT ({preserved}) INTO preserved_state;
  IF preserved_state->'tables' IS DISTINCT FROM before_state->'tables'
   OR preserved_state->'inventory' IS DISTINCT FROM before_state->'inventory'
   OR preserved_state->'sequences' IS DISTINCT FROM {literal(expected_sequences)}::jsonb THEN
   RAISE EXCEPTION 'current_keys_preexisting_state_not_preserved'; END IF;
  SELECT ({state}) INTO post_state;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',
   jsonb_build_object('scope',{literal(SCOPE)},'source_namespace_sha256',m->>'source_namespace_sha256',
    'mapping',m->'credential_receipt'->'records','material',m,'post_state',post_state));
 END IF;
 {_guard_sql(m)}
END
"""
    tag = "$lmm_current_keys_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "current_keys_sql_delimiter_collision")
    return f"""SET standard_conforming_strings=on;
SET timezone='UTC'; SET lock_timeout='5s'; SET statement_timeout='120s';
DO $fence$ BEGIN IF NOT pg_try_advisory_lock({FENCE}) THEN
 RAISE EXCEPTION 'current_keys_import_busy'; END IF; END $fence$;
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL row_security=off;
SET LOCAL search_path=pg_catalog,public;
DO {tag}{body}{tag};
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(m['migration_id'])},
 'material_sha256',{literal(digest(m))});
"""


def build_synthetic_transaction_sql(material, fixture_nonce):
    """For the separate PG test driver, never a production CLI option."""
    return _mechanics_transaction_sql(material, _test_fixture_guard(material, fixture_nonce))


def build_resolve_sql(m, original_application_name):
    """Read-only unknown-commit resolution. Receipt absence remains UNKNOWN.

    Resolving COMMITTED acknowledges the immutable transaction and exact key
    mapping. Later legitimate use may change unrelated data; the resolver does
    not require the whole target to still equal the transaction's post-state.
    A changed imported-key policy is reported as conflict for manual review.
    """
    validate_mechanics(m)
    need(type(original_application_name) is str and re.fullmatch(r"lmm-current-keys-[0-9a-f]{32}",
         original_application_name), "current_keys_attempt_id_invalid")
    instance = m["snapshot"]["instance"]
    body = f"""
DECLARE previous core_migration.import_runs%ROWTYPE;
BEGIN
 IF ({F.instance_sql()}) IS DISTINCT FROM {literal(instance)}::jsonb THEN
  RAISE EXCEPTION 'current_keys_resolve_instance_changed'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND application_name={literal(original_application_name)} AND pid<>pg_backend_pid()) THEN
  RAISE EXCEPTION 'current_keys_original_attempt_still_active'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])};
 IF FOUND THEN
  IF previous.manifest->'material' IS DISTINCT FROM {literal(m)}::jsonb
   OR previous.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR previous.source_archive_sha256<>{literal(m['source_archive_sha256'])}
   OR previous.mapping_sha256<>{literal(m['mapping_sha256'])}
   OR previous.manifest->'mapping' IS DISTINCT FROM {literal(m['credential_receipt']['records'])}::jsonb
   OR previous.manifest->>'source_namespace_sha256' IS DISTINCT FROM {literal(m['source_namespace_sha256'])}
  THEN RAISE EXCEPTION 'current_keys_resolve_receipt_conflict'; END IF;
  {_readback_sql(m)}
 END IF;
END
"""
    tag = "$lmm_resolve_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "current_keys_sql_delimiter_collision")
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL row_security=off;
DO {tag}{body}{tag};
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={literal(m['migration_id'])}) THEN 'COMMITTED' ELSE 'UNKNOWN' END,
 'migration_id',{literal(m['migration_id'])},'material_sha256',{literal(digest(m))},
 'automatic_retry_allowed',false);
COMMIT;
"""


def build_production_transaction_sql(material, evidence):
    context = require_current_reconciliation(material, evidence)
    validate_mechanics(material)
    observation = native_evidence_sql(material["target_projection"]["source_target_id_map"], material["migration_id"])
    guard = "IF (" + observation + ") IS DISTINCT FROM " + literal(context["native"]) + \
        "::jsonb THEN RAISE EXCEPTION 'current_keys_reconciled_native_evidence_changed'; END IF;\n"
    guard += W._readback_sql(context["wallet"], current_balances=True)
    return _mechanics_transaction_sql(material, "", guard)


def prepare_production(evidence, credential_review, *, migration_id, runtime_roles, output, encryption_key=None):
    need(migration_id == evidence.get("key_migration_id"), "current_keys_reconciliation_operation_id_mismatch")
    context = inspect_reconciliation(evidence)
    economic = validate_economic_review(evidence, context["facts"])
    need(credential_review["economic_review_sha256"] == digest(economic)
         and credential_review["evidence_sha256"] == evidence["economic_review"]["sha256"],
         "current_keys_credential_review_not_current_economic_review")
    material = prepare_mechanics(context["source"], context["target"], credential_review,
        snapshot=context["snapshot"], contract=context["contract"], migration_id=migration_id,
        source_namespace=evidence["source_namespace"], runtime_roles=runtime_roles,
        current_bindings=context["bindings"], dormant_bindings=context["dormant"], encryption_key=encryption_key)
    path = F.write_private(output, "current-verified-key-source.private.json", canonical(context["source"]) + "\n")
    material["source_files"] = {"source": file_ref(path)}
    material["target"] = {"instance": material["snapshot"]["instance"]}
    material["production_closure"] = {"scope": PRODUCTION_SCOPE,
        "evidence_sha256": digest(evidence), "facts": context["facts"]}
    validate_mechanics(material)
    return material


def validate_execution_approval(m, evidence, sql, approval):
    binding = {"format_version": 1, "scope": PRODUCTION_SCOPE,
        "decision": "append-reviewed-current-legacy-keys-with-native-rights-preserved",
        "material_sha256": digest(m), "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(),
        "reconciliation_sha256": digest(evidence), "economic_review_sha256": evidence["economic_review"]["sha256"]}
    need(type(approval) is dict and set(approval) == set(binding) | {"user_authorization_record"}
         and all(approval[k] == v for k, v in binding.items()), "current_keys_execution_approval_mismatch")
    record = read_ref(approval["user_authorization_record"])
    need(set(record) == set(binding) | {"user_id", "message_reference", "approved_at", "exact_user_message"}
         and all(record[k] == v for k, v in binding.items())
         and all(type(record[k]) is str and record[k].strip() for k in (
             "user_id", "message_reference", "approved_at", "exact_user_message")),
         "current_keys_execution_authorization_origin_missing")


def connection_binding(m, connection):
    owner, login = I.connection_roles(connection)
    expected = m["snapshot"]["instance"]
    need(owner == expected["role"] and login == expected["session_user"]
         and connection.get("database") == expected["database"] and connection.get("port") == expected["port"],
         "current_keys_connection_binding_changed")
    I.socket_connection(connection)


def execute(m, evidence, approval, connection, output):
    """One durable attempt. Every error after possible send stays UNKNOWN."""
    sql = build_production_transaction_sql(m, evidence)
    validate_execution_approval(m, evidence, sql, approval)
    need(read_ref(m["source_files"]["source"]) == inspect_reconciliation(evidence)["source"]
         and m["target"] == {"instance": m["snapshot"]["instance"]},
         "current_keys_alias_source_or_target_changed")
    connection_binding(m, connection)
    application = "lmm-current-keys-" + uuid.uuid4().hex
    attempt = {"format_version": 1, "operation": "apply", "application_name": application,
        "migration_id": m["migration_id"], "material_sha256": digest(m),
        "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(), "evidence_sha256": digest(evidence),
        "execution_approval": approval, "automatic_retry_allowed": False}
    F.write_private(output, "attempt.private.json", canonical(attempt) + "\n")
    F.write_private(output, "executed.private.sql", sql)
    outcome = {"status": "UNKNOWN", "migration_id": m["migration_id"], "material_sha256": digest(m)}
    try:
        code, raw = F.run_sql(connection, sql, output, application, label="apply")
        expected = {**outcome, "status": "COMMITTED"}
        if code == 0 and json.loads(raw, object_pairs_hook=P.unique_object) == expected:
            validate_mechanics(m)
            outcome = expected
    except Exception:
        pass
    try:
        F.write_private(output, "result.private.json", canonical(outcome) + "\n")
    except Exception:
        # A disk/publication failure after send cannot be reported as a
        # preflight BLOCKED result. Preserve the observed commit state.
        return {**outcome, "artifact_publication": "FAILED_REQUIRES_READONLY_RESOLUTION",
                "automatic_retry_allowed": False}
    if outcome["status"] == "COMMITTED":
        try:
            C.publish_committed_outputs(m, outcome, output)
        except Exception:
            # A publication error never changes a confirmed database commit to
            # a retryable failure. Resolve/readback before publishing again.
            return {**outcome, "artifact_publication": "FAILED_REQUIRES_READONLY_RESOLUTION"}
    return outcome


def resolve(m, attempt, connection, output):
    validate_mechanics(m)
    need(attempt.get("operation") == "apply" and attempt.get("migration_id") == m["migration_id"]
         and attempt.get("material_sha256") == digest(m), "current_keys_original_attempt_mismatch")
    connection_binding(m, connection)
    sql = build_resolve_sql(m, attempt["application_name"])
    F.write_private(output, "resolve.private.sql", sql)
    result = {"status": "UNKNOWN", "migration_id": m["migration_id"], "material_sha256": digest(m),
              "automatic_retry_allowed": False}
    try:
        code, raw = F.run_sql(connection, sql, output, "lmm-current-keys-resolve-" + uuid.uuid4().hex,
                             readonly=True, label="resolve")
        value = json.loads(raw, object_pairs_hook=P.unique_object)
        if code == 0 and value == {**result, "status": "COMMITTED"}:
            result = value
    except Exception:
        pass
    try:
        F.write_private(output, "result.private.json", canonical(result) + "\n")
    except Exception:
        return {**result, "artifact_publication": "FAILED_REQUIRES_READONLY_RESOLUTION"}
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("inspect", "source-fence-sql", "backup-verification-sql", "prepare", "apply", "resolve", "native-observation-sql"):
        p = sub.add_parser(name)
        p.add_argument("--output", required=True, type=Path)
        if name in {"inspect", "source-fence-sql", "backup-verification-sql", "prepare", "apply"}:
            p.add_argument("--evidence", required=True, type=Path)
        if name == "native-observation-sql":
            p.add_argument("--target-projection", required=True, type=Path)
        if name in {"apply", "resolve"}:
            p.add_argument("--material", required=True, type=Path)
            p.add_argument("--connection", required=True, type=Path)
        if name == "resolve":
            p.add_argument("--attempt", required=True, type=Path)
        if name == "apply":
            p.add_argument("--approval", required=True, type=Path)
        if name == "prepare":
            p.add_argument("--credential-review", required=True, type=Path)
            p.add_argument("--runtime-roles", required=True, type=Path)
            p.add_argument("--migration-id", required=True)
            p.add_argument("--encryption-key", type=Path)
    args = parser.parse_args(argv)
    try:
        frozen_dependencies()
        args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        I.private_backup_path(args.output, directory=True)
        if args.command == "native-observation-sql":
            target = C.load_json(args.target_projection)
            sql = native_observation_sql(target["source_target_id_map"])
            F.write_private(args.output, "native-observation.private.sql", sql)
            result = {"status": "READONLY_SQL_PREPARED", "target_queries": 0, "writes": 0}
        elif args.command == "resolve":
            result = resolve(C.load_json(args.material), C.load_json(args.attempt), C.load_json(args.connection), args.output)
        else:
            evidence = C.load_json(args.evidence)
            if args.command == "backup-verification-sql":
                snap = read_ref(evidence["target_snapshot"])
                contract = U.core78_contract(read_ref(evidence["core78_manifest"], parse=False))
                target = read_ref(evidence["target_projection"])
                sql = backup_verification_sql(snap, contract, target, evidence["key_migration_id"])
                F.write_private(args.output, "backup-verification.private.sql", sql)
                result = {"status": "READONLY_SQL_PREPARED", "target_queries": 0, "writes": 0}
            elif args.command == "source-fence-sql":
                _, proof, _, _, _ = read_source(evidence)
                F.write_private(args.output, "full-source-fence.private.sql", full_source_fence_sql(evidence["source_namespace"], proof))
                result = {"status": "READONLY_SQL_PREPARED", "source_queries": 0, "writes": 0}
            elif args.command == "inspect":
                context = inspect_reconciliation(evidence, require_ready=False)
                F.write_private(args.output, "reconciliation-facts.private.json", canonical(context["facts"]) + "\n")
                result = {"status": "BLOCKED" if context["facts"]["blockers"] else "FACTS_READY_FOR_REVIEW",
                    "blockers": context["facts"]["blockers"], "facts_sha256": digest(context["facts"]),
                    "production_authorized": False, "target_writes": 0}
            elif args.command == "prepare":
                material = prepare_production(evidence, C.load_json(args.credential_review), migration_id=args.migration_id,
                    runtime_roles=json.loads(C.private_bytes(args.runtime_roles)), output=args.output,
                    encryption_key=C.private_bytes(args.encryption_key) if args.encryption_key else None)
                sql = build_production_transaction_sql(material, evidence)
                F.write_private(args.output, "current-key-production.private.json", canonical(material) + "\n")
                F.write_private(args.output, "current-key-production.review.sql", sql)
                result = {"status": "READY_FOR_EXACT_EXECUTION_REVIEW", "production_authorized": False,
                    "material_sha256": digest(material), "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(),
                    "key_count": len(material["records"]), "target_writes": 0}
            else:
                result = execute(C.load_json(args.material), evidence, C.load_json(args.approval),
                                 C.load_json(args.connection), args.output)
        print(json.dumps(result, sort_keys=True))
        return 0 if result["status"] not in {"BLOCKED", "UNKNOWN"} else 2
    except Exception as exc:
        code = str(exc) if re.fullmatch(r"[A-Za-z0-9_]+", str(exc)) else "current_keys_private_input_invalid"
        print(json.dumps({"status": "BLOCKED", "blockers": [code], "production_authorized": False}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
