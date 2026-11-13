#!/usr/bin/env python3
"""Rebind committed legacy Key aliases to an actually observed recovery database."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys

import financial_import as F
import legacy_credential_import as C
import export_legacy_key_target_observation as W

H = F.IDENTITY
ALIAS_FILE = "legacy-api-key-wire-aliases.private.json"
RECEIPT_FILE = "credential-mapping-receipt.private.json"


def run(args) -> dict:
    raw_alias = C.private_bytes(args.alias_manifest)
    raw_receipt = C.private_bytes(args.credential_receipt)
    raw_report = C.private_bytes(args.commit_report)
    raw_target = C.private_bytes(args.target_receipt)
    H.require(F.sha(raw_target) == args.target_receipt_sha256, "key_rebind_target_receipt_digest_mismatch")
    alias, receipt = C.load_json(args.alias_manifest), C.load_json(args.credential_receipt)
    report, target = C.load_json(args.commit_report), C.load_json(args.target_receipt)
    rows = W.validate_alias_mapping(receipt, alias, raw_receipt)
    artifacts = report.get("artifacts", {})
    for name, raw, path in ((RECEIPT_FILE, raw_receipt, args.credential_receipt),
                            (ALIAS_FILE, raw_alias, args.alias_manifest)):
        artifact = artifacts.get(name, {})
        H.require(type(artifact) is dict and artifact.get("sha256") == F.sha(raw)
                  and Path(artifact.get("path", "")).resolve() == path.resolve(),
                  "key_rebind_commit_artifact_mismatch")
    H.require(report.get("phase") == "keys" and report.get("applied") is True
              and report.get("stats", {}).get("tokens") == len(rows)
              and report.get("stats", {}).get("alias_keys") == len(rows)
              and report.get("reconciliation", {}).get("legacy_tokens") == len(rows)
              and report.get("reconciliation", {}).get("core_api_keys") == len(rows)
              and report.get("reconciliation", {}).get("extra_core_keys") == 0
              and report.get("reconciliation", {}).get("mismatches") == {},
              "key_rebind_commit_report_invalid")
    H.require(target.get("application_or_worker_started") is False
              and target.get("source_archive_sha256") == alias["source_archive_sha256"],
              "key_rebind_target_not_offline_or_source_mismatch")
    connection = target["connection"]
    H.connection_roles(connection)
    sql = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" + H.IDENTITY_SQL + "\nCOMMIT;"
    actual = json.loads(H.psql(connection, sql, readonly=True,
                              private_dir=args.output.parent, label="key-alias-rebind-target"))
    H.verify_instance(actual, connection, target["instance_identity"], source=False)
    rebound_target = C.alias_target(actual)
    H.require(rebound_target != alias["target"], "key_rebind_target_unchanged")
    rebound = {**alias, "target": rebound_target}
    alias_text = C.canonical(rebound) + "\n"
    H.private_backup_path(args.output.parent, directory=True)
    args.output.mkdir(mode=0o700)
    H.write_private(args.output, ALIAS_FILE, alias_text)
    H.write_private(args.output, RECEIPT_FILE, raw_receipt.decode())
    receipt_out = {
        "format_version": 1,
        "scope": "legacy-api-key-alias-rebinding-v1",
        "status": "REBOUND_TO_OBSERVED_RECOVERY_TARGET",
        "source_archive_sha256": alias["source_archive_sha256"],
        "migration_id": report["migration_id"],
        "credential_count": len(rows),
        "credential_receipt_sha256": F.sha(raw_receipt),
        "original_alias_manifest_sha256": F.sha(raw_alias),
        "commit_report_sha256": F.sha(raw_report),
        "target_receipt_sha256": F.sha(raw_target),
        "alias_manifest_sha256": F.sha(alias_text.encode()),
        "original_target": alias["target"],
        "recovery_target": rebound_target,
        "database_writes": False,
        "key_mappings_changed": False,
    }
    H.write_private(args.output, "rebinding-receipt.private.json", C.canonical(receipt_out) + "\n")
    return {"status": receipt_out["status"], "credential_count": len(rows),
            "alias_manifest_sha256": receipt_out["alias_manifest_sha256"]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("alias-manifest", "credential-receipt", "commit-report", "target-receipt", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--target-receipt-sha256", required=True)
    args = parser.parse_args()
    try:
        print(C.canonical(run(args)))
    except (OSError, ValueError, TypeError, KeyError, H.MappingError) as error:
        print(C.canonical({"status": "REFUSED", "code": str(error)}), file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
