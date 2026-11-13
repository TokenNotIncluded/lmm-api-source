#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Read local fresh66 migration evidence and print a deterministic 66->67 plan.

No database, subprocess, network, import, upgrade or approval operation is
performed. Redirect stdout to retain the plan. A plan is NOT proof of completed
migration, a live preflight, authorization, or an executed upgrade. Original
receipts are never rewritten. Re-run with the same files and request ID to
obtain the same plan. Unknown commit outcomes require original-operation
recovery before handoff; applied:true and passed:true alone are not evidence.

Required files use existing formats: source and target receipts from
migrate-legacy-snapshot.py; snapshot from financial_import.snapshot_sql();
report from verify_legacy_cutover_rehearsal.py; its aliases and sibling
credential-mapping-receipt.private.json; UpgradeRequest from Core upgrade-db;
phase reports and adjacent {phase}.sql from legacy_cutover_import.py.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REVIEW_SHA256 = "0f62bc54c39562414224fc821becf90029f2becb9757b4bcb0018cc7fcb703e4"
PROCEDURE = "core-payment-discounts-v1"
ROW_CHECKS = {"source_rows", "target_rows", "missing", "extra", "different", "passed"}
REQUIRED_CHECKS = {"account_balances_registration_and_roles", "api_key_owner_digest_quota_and_lifecycle",
                   "mapped_log_field_projections", "discount_facts_source_binding"}


class Invalid(ValueError):
    pass


def require(condition, code):
    if not condition:
        raise Invalid(code)


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def load(path):
    with Path(path).open(encoding="utf-8") as stream:
        value = json.load(stream, object_pairs_hook=unique,
                          parse_constant=lambda _: (_ for _ in ()).throw(Invalid("nonfinite_json")))
    require(isinstance(value, dict), "json_object_required")
    return value


def sha(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value), "invalid_sha256")
    return value


def positive(value):
    require(type(value) is int and value > 0, "positive_integer_required")
    return value


def plan(args, root=ROOT):
    bindings = {}

    def bind(name, path, *, json_file=True):
        path = Path(path)
        require(path.is_file() and not path.is_symlink(), "regular_evidence_file_required")
        before = digest(path)
        value = load(path) if json_file else None
        require(digest(path) == before, "evidence_changed_during_read")
        bindings[name] = {"path": str(path.resolve()), "sha256": before}
        return value

    review = bind("fresh66_review", root / "scripts/reviewed-fresh66-module-split.json")
    require(bindings["fresh66_review"]["sha256"] == REVIEW_SHA256, "historical_review_changed")
    target_pin = bind("fresh67_pin", root / "services/core/src/core_schema/upgrade/payment_discounts_target.json")
    engine_path = root / "services/core/src/core_schema/upgrade/payment_discounts.rs"
    bind("upgrade_engine", engine_path, json_file=False)
    engine = engine_path.read_text()
    for field, expected in (("predecessor_manifest", review["compiled_manifest_sha256"]),
                            ("predecessor_catalog", review["fresh_catalog_sha256"])):
        require(f'{field}: "{expected}"' in engine, "upgrade_predecessor_differs_from_fresh66")
    require("predecessor_count: 66" in engine and "target_count: 67" in engine
            and f'PROCEDURE: &str = "{PROCEDURE}"' in engine, "upgrade_engine_contract_changed")
    source = bind("source_receipt", args.source_receipt)
    bind("source_archive", args.archive, json_file=False)
    archive_hash = bindings["source_archive"]["sha256"]
    require(source.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified"
            and source.get("archive", {}).get("sha256") == archive_hash, "source_archive_binding_mismatch")
    target = bind("target_receipt", args.target_receipt)
    require(target.get("status") == "initialized-empty-identity-target"
            and target.get("source_archive_sha256") == archive_hash, "target_receipt_source_mismatch")
    contract = target["contract"]
    require(contract.get("import_profile") == "fresh66-module-split-identity6-complete-seeds-v1"
            and contract.get("compiled_manifest_sha256") == review["compiled_manifest_sha256"]
            and contract.get("fresh_catalog_sha256") == review["fresh_catalog_sha256"], "target_not_reviewed_fresh66")
    snapshot = bind("target_snapshot", args.target_snapshot)
    require(snapshot.get("catalog_fingerprint_sha256") == review["fresh_catalog_sha256"]
            and snapshot.get("component_fingerprints") == contract.get("component_fingerprints")
            and isinstance(snapshot.get("component_fingerprints"), dict)
            and len(snapshot["component_fingerprints"]) == 66, "snapshot_not_fresh66")
    identity = snapshot["instance"]
    receipt_identity = target["instance_identity"]
    for field in ("database_oid", "server_instance_nonce", "server_version_num", "postmaster_started_at"):
        require(field in identity and str(identity[field]) == str(receipt_identity.get(field)), "target_instance_binding_mismatch")
    sha(identity["server_instance_nonce"])
    aliases = bind("aliases", args.aliases)
    bind("credential_receipt", Path(args.aliases).with_name("credential-mapping-receipt.private.json"))
    require(aliases.get("version") == 1 and aliases.get("scope") == "legacy-api-key-wire-aliases-v1"
            and aliases.get("source_archive_sha256") == archive_hash
            and aliases.get("credential_receipt_sha256") == bindings["credential_receipt"]["sha256"]
            and aliases.get("target") == {"database": identity["database"], "database_oid": identity["database_oid"],
                                        "instance_nonce": identity["server_instance_nonce"]}, "aliases_binding_mismatch")
    report = bind("reconciliation", args.reconciliation)
    verifier = root / "scripts/verify_legacy_cutover_rehearsal.py"
    bind("verifier", verifier, json_file=False)
    inputs = report["inputs"]
    require(report.get("scope") == "read-only-available-core-phase-reconciliation"
            and inputs.get("source_archive_sha256") == archive_hash
            and inputs.get("aliases_sha256") == bindings["aliases"]["sha256"]
            and inputs.get("verifier_sha256") == bindings["verifier"]["sha256"], "reconciliation_binding_mismatch")
    positive(inputs["observed_at"])
    checks = report["checks"]
    require(isinstance(checks, dict) and REQUIRED_CHECKS <= checks.keys(), "reconciliation_checks_missing")
    numeric_checks = []
    for name, check in checks.items():
        require(isinstance(check, dict) and check.get("passed") is True, "reconciliation_failed")
        if ROW_CHECKS <= check.keys():
            require(all(type(check[k]) is int and check[k] >= 0 for k in ROW_CHECKS - {"passed"}), "invalid_comparison_counts")
            require(check["source_rows"] == check["target_rows"] and
                    all(check[k] == 0 for k in ("missing", "extra", "different")), "reconciliation_numeric_mismatch")
            numeric_checks.append(name)
    require(REQUIRED_CHECKS <= set(numeric_checks), "boolean_only_reconciliation_rejected")
    gaps = report["source_gaps"]
    require(isinstance(gaps, dict) and all(type(n) is int and n >= 0 for n in gaps.values()), "invalid_source_gaps")
    expected_status = "SCOPED_PASS_WITH_GAPS" if any(gaps.values()) else "SCOPED_PASS"
    require(report.get("status") == expected_status, "reconciliation_status_mismatch")
    phases = []
    for index, path in enumerate(args.phase_report):
        phase = bind(f"phase_report_{index}", path)
        name = phase.get("phase")
        require(isinstance(name, str) and re.fullmatch(r"[a-z_]+", name) and name not in phases, "invalid_or_duplicate_phase")
        require(phase.get("status") not in ("UNKNOWN", "unknown"), "phase_commit_unknown")
        require(phase.get("applied") is True and isinstance(phase.get("migration_id"), str), "phase_not_reported_applied")
        bind(f"phase_sql_{index}", Path(path).with_name(name + ".sql"), json_file=False)
        require(phase.get("plan_sha256") == bindings[f"phase_sql_{index}"]["sha256"], "phase_sql_hash_mismatch")
        phases.append(name)
    request = bind("upgrade_request", args.upgrade_request)
    require(set(request) == {"version", "procedure", "upgrade_id", "database", "roles", "predecessor_manifest_sha256",
                             "predecessor_catalog_sha256", "successor_manifest_sha256"}, "upgrade_request_shape")
    require(type(request["version"]) is int and request["version"] == 1 and request["procedure"] == PROCEDURE
            and re.fullmatch(r"[A-Za-z0-9_.-]{16,96}", request["upgrade_id"]), "upgrade_request_contract")
    db = request["database"]
    require(set(db) == {"name", "oid", "system_identifier", "server_version_num"}, "upgrade_database_shape")
    require(db["name"] == identity["database"] and positive(db["oid"]) == identity["database_oid"]
            and db["system_identifier"] == identity["system_identifier"]
            and type(db["server_version_num"]) is int and db["server_version_num"] == int(identity["server_version_num"])
            and 180000 <= db["server_version_num"] < 190000, "upgrade_database_binding_mismatch")
    require(isinstance(db["system_identifier"], str) and re.fullmatch(r"[0-9]{1,20}", db["system_identifier"])
            and 0 < int(db["system_identifier"]) <= 2**64-1, "invalid_system_identifier")
    roles = request["roles"]
    require(set(roles) == {"installer", "owner", "runtime", "extensions"}
            and all(isinstance(v, str) and re.fullmatch(r"[a-z_][a-z_0-9]{0,62}", v) for v in roles.values())
            and len(set(roles.values())) == 4, "invalid_upgrade_roles")
    connection = target["connection"]
    require(roles["owner"] == connection["role"] == identity["role"]
            and roles["installer"] == connection["login_role"] == identity["session_user"]
            and connection["database"] == db["name"], "upgrade_role_binding_mismatch")
    require(request["predecessor_manifest_sha256"] == review["compiled_manifest_sha256"]
            and request["predecessor_catalog_sha256"] == review["fresh_catalog_sha256"]
            and request["successor_manifest_sha256"] == sha(target_pin["manifest_sha256"]), "upgrade_pin_mismatch")
    sha(target_pin["catalog_sha256"])
    # Recheck every input, including the archive, before emitting any plan.
    require(all(digest(v["path"]) == v["sha256"] for v in bindings.values()), "evidence_changed_during_read")
    return {"protocol": "fresh66-payment-discounts-handoff.v1", "status": "PLAN_ONLY_NOT_EXECUTED",
            "artifacts": bindings, "upgrade_request": request, "expected_successor_catalog_sha256": target_pin["catalog_sha256"],
            "numerically_consistent_comparisons": sorted(numeric_checks), "reported_phases": sorted(phases),
            "source_gaps": gaps,
            "steps": ["Complete and reconcile all imports with preserved fresh66 tools; resolve every original UNKNOWN operation.",
                      "Recheck actual target identity, 66 catalog, import COMMIT receipts and offline session fence.",
                      "Use the frozen fresh67 owner installer upgrade-db with the exact bound request; retain its original ID on UNKNOWN.",
                      "Read the upgrade receipt and verify 67 catalog, runtime grants and unchanged imported data/ledger; retain original 66 receipts."],
            "remaining": ["Local hashes are bindings, not authenticated or live COMMIT evidence.",
                          "Phase reports do not prove complete phase coverage or durable commits.",
                          "Reconciliation scope and source gaps are not full migration/business acceptance.",
                          "No live database identity, fence, credential/config authority, upgrade or post-upgrade preservation checked."]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source-receipt", "archive", "target-receipt", "target-snapshot", "aliases", "reconciliation", "upgrade-request"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--phase-report", required=True, action="append", type=Path)
    args = parser.parse_args(argv)
    try:
        result = plan(args)
    except (Invalid, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"status": "REJECTED", "code": str(error) if isinstance(error, Invalid) else "evidence_unreadable_or_incomplete"}), file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
