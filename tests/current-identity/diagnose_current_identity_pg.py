#!/usr/bin/env python3
"""One bounded, rollback-only trace of the failed synthetic identity writer.

This is a diagnostic collector, not a passing migration regression or a
production entry point. It requires the unchanged 200c3134 module and a fresh
shared Core78 synthetic fixture. All original assertions stay in place. No
COMMIT acknowledgement proxy is used and no failed write is retried.

Only synthetic user id/active/auth_version and an operation label are emitted
as trace rows. ON_ERROR_STOP stays enabled: a refusal exits psql, closing its
transaction, followed by independent full-table rollback verification. If the
writer unexpectedly reaches its end, an explicit ROLLBACK replaces COMMIT.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import uuid

import migrate_current_identity as M
import test_current_identity_pg as T


MODULE_SHA256 = "200c31344a84fcc0770f0568056b78baefbbd97a2e780ef6725151cf463d9f26"
TRACE_PREFIX = "LMM_IDENTITY_AUTH_TRACE "


def trace_sql(material, fixture_nonce, fixture_user_id):
    M.need(hashlib.sha256(Path(M.__file__).read_bytes()).hexdigest() == MODULE_SHA256,
           "diagnostic_requires_exact_failed_module")
    M.need(material["source_kind"] == "synthetic-test-facts", "diagnostic_synthetic_source_required")
    ops = material["plan"]["operations"]
    M.need(0 < len(ops) <= 200, "diagnostic_operation_bound_exceeded")
    users = sorted({r["target_user_id"] for r in material["plan"]["mapping"]} | {fixture_user_id})
    M.need(len(users) <= 32 and all(type(uid) is int and uid > 0 for uid in users),
           "diagnostic_user_bound_exceeded")
    users_sql = ",".join(str(uid) for uid in users)

    def notice(operation):
        return ("RAISE NOTICE '" + TRACE_PREFIX + "%', jsonb_build_object('operation'," + M.literal(operation)
                + ",'users',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'active',active,"
                  "'auth_version',auth_version) ORDER BY id),'[]'::jsonb) FROM core_identity.users "
                + "WHERE id IN (" + users_sql + ")));" )

    original = M.build_sql(material, fixture_nonce=fixture_nonce)
    writes = M.write_sql(material)
    M.need(original.count(writes) == 1, "diagnostic_write_segment_not_unique")
    traced, index = [notice("before_mutations")], 0
    for line in writes.splitlines():
        traced.append(line)
        if line.startswith(("INSERT INTO ", "UPDATE ")):
            M.need(index < len(ops), "diagnostic_mutation_count_changed")
            operation = ops[index]
            prefix = ("INSERT INTO " if operation["before"] is None else "UPDATE ") + M.F.relation(operation["table"])
            M.need(line.startswith(prefix), "diagnostic_operation_order_changed")
            label = str(index) + ":" + operation["table"] + ":" + operation["reason"]
            traced.append(notice(label))
            index += 1
        else:
            M.need(line.startswith("IF nextval("), "diagnostic_unrecognized_generated_statement")
    M.need(index == len(ops), "diagnostic_mutation_count_changed")
    traced.append(notice("after_mutations_before_constraints_and_readback"))
    sql = original.replace(writes, "\n".join(traced))
    # Leave both exact original readback branches and every other guard intact.
    readback = M.readback_sql(material)
    M.need(original.count(readback) == sql.count(readback) == 2, "diagnostic_changed_readback")
    suffix = re.search(r"(\$current_identity_[0-9a-f]{24}\$); COMMIT;\nSELECT jsonb_build_object\('status','COMMITTED',", sql)
    M.need(suffix is not None and sql.count("; COMMIT;\nSELECT jsonb_build_object('status','COMMITTED',") == 1,
           "diagnostic_transaction_tail_changed")
    # This tail is unreachable on the preserved failing assertion. psql then
    # exits with ON_ERROR_STOP=1 and PostgreSQL rolls back on connection close.
    sql = sql[:suffix.start()] + suffix.group(1) + "; ROLLBACK;\n"
    sql = "SET client_min_messages=notice;\n" + sql
    M.need("; COMMIT;" not in sql and "'status','COMMITTED'" not in sql,
           "diagnostic_commit_path_not_removed")
    return sql, {"original_sql_sha256": hashlib.sha256(original.encode()).hexdigest(),
                 "diagnostic_sql_sha256": hashlib.sha256(sql.encode()).hexdigest(),
                 "module_sha256": MODULE_SHA256, "original_readback_preserved": True,
                 "original_all_guards_preserved": True, "commit_path_removed": True,
                 "operation_count": len(ops), "traced_user_count": len(users)}


def parse_trace(stderr):
    result = []
    for line in stderr.splitlines():
        if TRACE_PREFIX not in line:
            continue
        item = json.loads(line.split(TRACE_PREFIX, 1)[1])
        M.need(set(item) == {"operation", "users"} and type(item["operation"]) is str,
               "diagnostic_trace_shape_changed")
        M.need(type(item["users"]) is list and len(item["users"]) <= 32, "diagnostic_trace_user_limit")
        for user in item["users"]:
            M.need(set(user) == {"id", "active", "auth_version"}
                   and type(user["id"]) is int and type(user["active"]) is bool
                   and type(user["auth_version"]) is int, "diagnostic_trace_contains_unapproved_fields")
        result.append(item)
    return result


def trigger_catalog_sql():
    tables = ",".join(M.literal(t) for t in M.MUTABLE)
    return """SELECT coalesce(jsonb_agg(jsonb_build_object(
 'table_name',n.nspname||'.'||c.relname,'trigger_name',t.tgname,
 'trigger_definition',pg_get_triggerdef(t.oid),'function_definition',pg_get_functiondef(t.tgfoid))
 ORDER BY n.nspname,c.relname,t.tgname),'[]'::jsonb)
 FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE NOT t.tgisinternal AND n.nspname||'.'||c.relname IN (""" + tables + ")"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    os.umask(0o077)
    M.need(hashlib.sha256(Path(M.__file__).read_bytes()).hexdigest() == MODULE_SHA256,
           "diagnostic_requires_exact_failed_module")
    # The shared constructor checks exact Core78 catalog, physical identity,
    # private Unix socket, nonce/comment and strict owner/installer permissions
    # before the ordinary synthetic fixture seed is allowed.
    regression = T.Regression(M.private_json(args.fixture), args.output)
    regression.seed_sessions_and_budget()
    regression.seed_identity()
    migration = "synthetic-identity-trace-" + uuid.uuid4().hex[:12]
    material = regression.prepare_identity(migration)
    before = regression.state(migration)
    catalog = regression.query(trigger_catalog_sql())
    M.F.write_private(args.output, "trigger-catalog.private.json", M.canonical(catalog) + "\n")
    sql, construction = trace_sql(material, regression.nonce, regression.fixture_user_id)
    app = "lmm-current-identity-trace-" + uuid.uuid4().hex
    code, _, label = regression.run_sql(sql, expect_ok=False, app=app)
    stderr = M.private_bytes(args.output / (label + ".stderr.private.txt")).decode()
    trace = parse_trace(stderr)
    M.F.write_private(args.output, "auth-version-trace.public.json", M.canonical(trace) + "\n")
    # A second real connection proves the failed writer and its session lock
    # ended. Full inventory hashes prove rollback of every application table.
    active = regression.query("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=" + M.literal(app) + ")")
    after = regression.state(migration)
    tables_unchanged = after["tables"] == before["tables"]
    receipt_absent = regression.query("SELECT NOT EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id="
                                      + M.literal(migration) + ")")
    sequences = {name: {"before": before["sequences"][name], "after": after["sequences"][name]}
                 for name in M.SEQUENCES}
    result = {"status": "DIAGNOSTIC_COMPLETE_MIGRATION_STILL_REJECTED" if code else "DIAGNOSTIC_COMPLETE_UNEXPECTED_NO_REFUSAL",
              "migration_passed": False, "production_access": False, "actual_psql_exit": code,
              "expected_readback_refusal_observed": "identity_affected_row_readback_mismatch" in stderr,
              "trace_rows": len(trace), "construction": construction,
              "writer_connection_still_active": active, "all_application_tables_unchanged": tables_unchanged,
              "current_identity_receipt_absent": receipt_absent,
              "sequences_not_reset": sequences, "trace_file": "auth-version-trace.public.json"}
    M.F.write_private(args.output, "diagnostic-result.public.json", M.canonical(result) + "\n")
    print(json.dumps(result, sort_keys=True))
    M.need(not active and tables_unchanged and receipt_absent, "diagnostic_rollback_not_proven")
    M.need(code != 0 and result["expected_readback_refusal_observed"], "diagnostic_expected_refusal_not_observed")
    M.need(len(trace) == len(material["plan"]["operations"]) + 2, "diagnostic_trace_incomplete")
    for name in M.SEQUENCES:
        M.need(after["sequences"][name]["last_value"] == before["sequences"][name]["last_value"]
               + material["plan"]["new_user_count"], "diagnostic_sequence_effect_unexpected")


if __name__ == "__main__":
    main()
