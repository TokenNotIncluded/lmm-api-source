#!/usr/bin/env python3
"""Explicit private preview/apply/resolve entry point for current identities.

Source-bundle paths select existing immutable, actual observations. Preview
never emits approval/fence assertions and never writes a database. Apply first
rebuilds the plan from those files, checks the real final source fence and
independent authorization, and executes one transaction. A transport/process
error is UNKNOWN; this driver never retries or reopens either writer.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import sys
import uuid

import migrate_current_identity as M


def source_bundle(path):
    spec = M.private_json(path)
    M.need(set(spec) == {"format_version", "scope", "source", "oauth", "restore", "original_mapping", "routing"}
           and spec["format_version"] == 1 and spec["scope"] == "current-identity-source-inputs-v1",
           "identity_source_bundle_invalid")
    bundle = M.load_source(*(Path(spec[k]) for k in ("source", "oauth", "restore", "original_mapping")))
    M.load_routing(bundle, spec["routing"])
    bundle["files"]["source_bundle"] = M.file_ref(path)
    return bundle


def tool_files():
    modules = (M, M.U, M.F, M.C, M.I, M.O, M.U.P)
    paths = {Path(x.__file__).resolve() for x in modules} | {Path(__file__).resolve()}
    return {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(paths)}


def prepare_inputs(bundle_path, snapshot_path, manifest_path, roles_path, migration_id):
    refs = {k: M.file_ref(p) for k, p in (("source_bundle", bundle_path), ("target_snapshot", snapshot_path),
            ("compiled_manifest", manifest_path), ("runtime_roles", roles_path))}
    bundle = source_bundle(bundle_path)
    contract = M.U.core78_contract(M.private_bytes(manifest_path))
    result = M.prepare(bundle, M.private_json(snapshot_path), contract, migration_id, M.private_json(roles_path))
    result["input_files"] = refs
    result["tool_files"] = tool_files()
    return result


def rebuild(material):
    M.validate_material(material)
    M.verify_files(material["input_files"])
    M.verify_files(material["source_files"])
    M.need(material["tool_files"] == tool_files(), "identity_execution_tool_bytes_changed")
    paths = material["input_files"]
    actual = prepare_inputs(*(Path(paths[k]["path"]) for k in ("source_bundle", "target_snapshot", "compiled_manifest", "runtime_roles")),
                            material["migration_id"])
    M.need(actual == material, "identity_private_source_rebuild_mismatch")


def write(directory, name, value):
    return M.F.write_private(directory, name, value if type(value) is str else M.canonical(value) + "\n")


def output_directory(path):
    M.need(not path.exists(), "identity_output_directory_already_exists")
    path.mkdir(mode=0o700, parents=False)
    return path


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("preview")
    p.add_argument("--source-bundle", type=Path, required=True)
    p.add_argument("--target-snapshot", type=Path, required=True)
    p.add_argument("--core78-manifest", type=Path, required=True)
    p.add_argument("--runtime-roles", type=Path, required=True)
    p.add_argument("--migration-id", required=True)
    p.add_argument("--output", type=Path, required=True)
    for name in ("apply", "resolve"):
        p = sub.add_parser(name)
        p.add_argument("--material", type=Path, required=True)
        p.add_argument("--connection", type=Path, required=True)
        p.add_argument("--output", type=Path, required=True)
        if name == "apply":
            p.add_argument("--source-review", type=Path, required=True)
            p.add_argument("--authorization", type=Path, required=True)
        else:
            p.add_argument("--original-application-name", required=True)
    args = parser.parse_args(argv)
    os.umask(0o077)
    out = output_directory(args.output)
    if args.command == "preview":
        material = prepare_inputs(args.source_bundle, args.target_snapshot, args.core78_manifest, args.runtime_roles, args.migration_id)
        write(out, "identity.material.private.json", material)
        write(out, "identity.preview.private.sql", M.build_sql(material))
        write(out, "identity-final-source-fence.sql", M.final_fence_observation_sql(material["source_namespace"]))
        # The mapping is explicitly a proposal until the immutable target
        # transaction receipt and actual target readback establish completion.
        write(out, "identity-map.proposal.private.json", {"format_version": 1, "scope": M.SCOPE,
            "status": "PROPOSED_NOT_APPLIED", "source_archive_sha256": material["source_archive_sha256"],
            "source_namespace_sha256": material["source_namespace_sha256"],
            "mapping_hash_domain": material["mapping_hash_domain"], "identity_map_sha256": material["identity_map_sha256"],
            "mapping": material["plan"]["mapping"]})
        write(out, "summary.public.json", M.summary(material))
        print(json.dumps(M.summary(material)))
        return
    material = M.private_json(args.material)
    connection = M.private_json(args.connection)
    if args.command == "resolve":
        # The receipt may be resolved after legitimate subsequent target work;
        # do not rebuild a stale preview or require runtime roles still fenced.
        sql = M.resolve_sql(material, args.original_application_name)
        app = "lmm-current-identity-resolve-" + uuid.uuid4().hex
    else:
        rebuild(material)
        sql = M.build_sql(material)
        M.validate_review_and_authorization(material, sql, args.source_review, args.authorization)
        app = "lmm-current-identity-" + uuid.uuid4().hex
        write(out, "authorization.bindings.private.json", {"source_review": M.file_ref(args.source_review),
            "authorization": M.file_ref(args.authorization), "material": M.file_ref(args.material),
            "sql_sha256": hashlib.sha256(sql.encode()).hexdigest()})
    write(out, "attempt.private.json", {"status": "DISPATCH_PENDING", "application_name": app,
        "operation": args.command, "started_at": datetime.now(timezone.utc).isoformat(),
        "material_sha256": M.digest(material), "automatic_retry_allowed": False})
    write(out, "identity-operation.private.sql", sql)
    result = {"status": "UNKNOWN", "application_name": app, "automatic_retry_allowed": False}
    try:
        code, raw = M.F.run_sql(connection, sql, out, app, label="identity-" + args.command)
        result["psql_exit"] = code
        if code == 0:
            rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
            M.need(len(rows) == 1 and rows[0].get("status") in ("COMMITTED", "UNKNOWN"), "identity_sql_acknowledgement_unrecognized")
            if args.command == "apply":
                M.need(rows[0].get("migration_id") == material["migration_id"]
                       and rows[0].get("material_sha256") == M.digest(material), "identity_commit_receipt_binding_mismatch")
            result.update(rows[0])
    except BaseException as error:
        result["transport_or_verification_error_class"] = type(error).__name__
        write(out, "outcome.private.json", result)
        print(json.dumps(result))
        raise SystemExit(2) from None
    write(out, "outcome.private.json", result)
    print(json.dumps(result))
    if result["status"] != "COMMITTED":
        raise SystemExit(2)


if __name__ == "__main__":
    main()
