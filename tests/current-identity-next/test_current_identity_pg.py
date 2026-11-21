#!/usr/bin/env python3
"""Real used-Core78 identity regression on a fresh disposable shared fixture.

Uses the reviewed Core78 fixture/permissions and real COMMIT-loss Unix relay
from the key regression. This driver seeds only synthetic identities. It never
opens a production connection, accepts a production database name, or claims
real Google authentication. Run it in its own new fixture, not the key test DB.
"""
from copy import deepcopy
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import uuid

import migrate_current_identity as M
import test_current_identity as T
import test_migrate_current_runtime_keys_pg as K


def check(ok, message):
    if not ok:
        raise AssertionError(message)


def shifted_fixture():
    """Keep the harness's funded user1 intact and use another identity cohort."""
    b, p, _ = T.inputs()
    for row in b["source"]["data"]["users"]:
        row["id"] += 100
    b["source"]["source_identity_facts_sha256"] = M.digest(M.identity_facts(b["source"]["data"]))
    b["source"]["current_policy_records"] = M.mapped_current(b["source"]["data"], b["source"]["source_archive_sha256"])
    for row in b["original"]["records"]:
        row["user_id"] += 100
        row["account_id"] += 100
    b["original"]["mapping_sha256"] = M.digest({"source_sha256": "a" * 64, "records": b["original"]["records"]})
    for row in b["oauth"]["projection"]["users"]:
        row["id"] += 100
    for row in b["oauth"]["projection"]["oauth_bindings"]:
        row["user_id"] += 100
    for row in b["oauth_rows"]:
        row["source_user_id"] += 100
    for row in b["routing"]["facts"]["users"]:
        row["id"] += 100
    for name in ("users", "accounts", "profiles", "developer", "passwords", "oauth", "account_policies", "routing_grants"):
        for row in p[name]:
            for key in ("id", "user_id", "personal_account_id", "account_id"):
                if key in row:
                    row[key] += 100
            if "login_name" in row:
                row["login_name"] = "legacy-" + str(row["user_id"])
    original = p["original_imports"][0]
    original["mapping_sha256"] = b["original"]["mapping_sha256"]
    original["manifest"].update(mapping_sha256=b["original"]["mapping_sha256"],
        mapping_file_sha256=hashlib.sha256((M.canonical(b["original"]) + "\n").encode()).hexdigest(),
        source_target_id_map_sha256=hashlib.sha256(M.I.id_map_bytes(b["original"]["records"]).encode()).hexdigest())
    for receipt in p["history_imports"]:
        manifest = receipt["manifest"]
        for row in manifest["mapping"]:
            for key in ("source_user_id", "target_user_id", "target_personal_account_id"):
                row[key] += 100
        for row in manifest["source_user_facts"]:
            row["facts"]["id"] += 100
            row["source_fact_sha256"] = M.digest(row["facts"])
    M.attach_routing(b, b["routing"]["facts"])
    M.validate_oauth_projection(b["source"], b["oauth"])
    return b, p


class Regression(K.Regression):
    def seed_identity(self):
        self.bundle, p = shifted_fixture()
        statements = ["BEGIN ISOLATION LEVEL SERIALIZABLE;"]
        for key, table in (("accounts", "core_identity.accounts"), ("users", "core_identity.users"),
                           ("profiles", "core_identity.user_profiles"), ("developer", "core_identity.developer_access"),
                           ("account_policies", "core_billing.account_policies"), ("oauth", "core_identity.oauth_identities")):
            for row in p[key]:
                override = " OVERRIDING SYSTEM VALUE" if key == "accounts" else ""
                statements.append(f"INSERT INTO {table}(" + ",".join(row) + ")" + override + " VALUES("
                                  + ",".join(M.literal(v) for v in row.values()) + ");")
        for row in p["passwords"]:
            statements.append("INSERT INTO core_identity.password_logins(user_id,login_name,password_hash) VALUES("
                              + ",".join(M.literal(v) for v in (row["user_id"], row["login_name"], T.PASSWORD)) + ");")
        for receipt in p["original_imports"] + p["history_imports"]:
            statements.append("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES("
                + ",".join(M.literal(v) for v in (receipt["migration_id"], receipt.get("source_archive_sha256", "b" * 64),
                    receipt.get("mapping_sha256", M.digest(receipt["manifest"])), receipt["manifest"])) + ");")
        # Intentional normal nextval gaps set up an already-used sequence. No
        # writer or test uses setval/RESTART to hide consumed allocations.
        for sequence in M.SEQUENCES:
            statements.append("SELECT nextval(" + M.literal(sequence) + "::regclass) FROM generate_series(1,200);")
        statements.append("COMMIT;")
        self.run_sql("\n".join(statements))
        self.completed.append("used_target_contains_independent_native_root_history_and_original_mapping")

    def prepare_identity(self, migration):
        snapshot = self.query(M.snapshot_sql(self.contract, self.inventory, migration))
        material = M.prepare(self.bundle, snapshot, self.contract, migration, self.fixture["runtime_roles"])
        M.F.write_private(self.output, migration + "-" + str(self.counter) + ".material.private.json", M.canonical(material) + "\n")
        return material

    def identity_reject(self, sql, expected):
        before = self.state()
        code, _, label = self.run_sql(sql, expect_ok=False)
        check(code != 0, "unsafe identity operation unexpectedly succeeded")
        stderr = M.C.private_bytes(self.output / (label + ".stderr.private.txt")).decode()
        check(expected in stderr, "unexpected refusal; inspect " + label)
        check(self.state() == before, "rejected identity plan modified rows or consumed IDs")

    def concurrent_identity_fence(self, material):
        env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        env.update(M.I.socket_connection(self.connection))
        env.update(PGAPPNAME="identity-fence-" + uuid.uuid4().hex,
                   PGOPTIONS=M.I.psql_options(self.connection, readonly=False))
        proc = subprocess.Popen(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True, env=env)
        try:
            proc.stdin.write(f"SELECT pg_advisory_lock({M.U.FENCE}); SELECT pg_sleep(20);\n")
            proc.stdin.close()
            for _ in range(50):
                if self.query(f"SELECT NOT pg_try_advisory_xact_lock({M.U.FENCE})"):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("actual competing lock did not become ready")
            self.identity_reject(M.build_sql(material, fixture_nonce=self.nonce), "identity_import_busy")
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
        self.completed.append("actual_competing_writer_refused_before_nextval")

    def proxy_identity_attempt(self, material, before_commit):
        conn = deepcopy(self.connection)
        socket_path = Path(conn["socket_directory"]) / (".s.PGSQL." + str(conn["port"]))
        directory = self.output / ("identity-proxy-" + uuid.uuid4().hex[:8])
        app = "lmm-current-identity-" + uuid.uuid4().hex
        with K.CommitDropProxy(socket_path, directory, conn["port"], before_commit=before_commit):
            conn["socket_directory"] = str(directory)
            code, _, _ = self.run_sql(M.build_sql(material, fixture_nonce=self.nonce), connection=conn, app=app, expect_ok=False)
            check(code != 0, "a dropped commit was unexpectedly acknowledged")
        for _ in range(50):
            active = self.query("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=" + M.literal(app) + ")")
            if not active:
                break
            time.sleep(0.1)
        check(not active, "original attempt remains active; no retry allowed")
        return self.query(M.resolve_sql(material, app))

    def run_identity(self):
        self.seed_sessions_and_budget()
        self.seed_identity()
        migration = "synthetic-current-identity-" + uuid.uuid4().hex[:12]
        material = self.prepare_identity(migration)
        sql = M.build_sql(material, fixture_nonce=self.nonce)
        self.identity_reject(M.build_sql(material, fixture_nonce="f" * 64), "identity_synthetic_fixture_identity_changed")
        self.concurrent_identity_fence(material)
        # Actual unrelated budget revision changes invalidate the whole preview.
        self.run_sql("BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE core_charging.budgets SET limit_credits=10000,revision=revision+1 WHERE id="
                     + str(self.native_budget_id) + "; COMMIT;")
        self.identity_reject(sql, "identity_target_state_changed")
        self.completed.append("unrelated_native_state_CAS_refused_without_ID_consumption")
        material = self.prepare_identity(migration)
        sql = M.build_sql(material, fixture_nonce=self.nonce)
        before = self.state(migration)
        absent = self.proxy_identity_attempt(material, True)
        check(absent["status"] == "UNKNOWN" and absent["automatic_retry_allowed"] is False, "absence was called rollback")
        after = self.state(migration)
        check(after["tables"] == before["tables"], "pre-COMMIT disconnect failed to roll back identity rows")
        check(all(after["sequences"][s]["last_value"] > before["sequences"][s]["last_value"] for s in M.SEQUENCES),
              "normal nonrollbackable allocation gaps missing")
        self.identity_reject(sql, "identity_target_state_changed")
        self.completed.append("real_disconnect_rolls_back_rows_preserves_sequence_gaps_and_unknown_outcome")
        fresh = self.prepare_identity(migration)
        check(fresh["plan"]["first_user_id"] > material["plan"]["first_user_id"], "new preview reused allocated IDs")
        committed = self.proxy_identity_attempt(fresh, False)
        check(committed["status"] == "COMMITTED" and committed["automatic_retry_allowed"] is False
              and committed["current_identity_readback_matches"] is True, "lost COMMIT acknowledgement did not resolve")
        state = self.state(migration)
        result = self.query(M.build_sql(fresh, fixture_nonce=self.nonce))
        check(result["status"] == "COMMITTED" and self.state(migration) == state, "exact replay allocated or changed state")
        root = self.query("SELECT jsonb_build_object('active',active,'auth_version',auth_version,'role',platform_role) FROM core_identity.users WHERE id=101")
        check(root == {"active": True, "auth_version": 3, "role": "superadmin"}, "native root authority was overwritten")
        full = self.query(M.projection_sql())
        mapper = {r["source_user_id"]: r for r in fresh["plan"]["mapping"]}
        history = next(r for r in full["users"] if r["id"] == 109)
        # Actual diagnostic run44951870890 established that activating the
        # dormant user and opening its existing password gate are two native
        # generation changes: v1 -> v2 -> v3, without disabling either trigger.
        check(history["active"] and history["auth_version"] == 3, "history generation stages were not preserved")
        receipt = self.query("SELECT manifest FROM core_migration.import_runs WHERE migration_id=" + M.literal(migration))
        lineage = next(r for r in receipt["auth_version_lineage"] if r["source_user_id"] == 108)
        actual = next(r for r in receipt["actual_identity_authority"] if r["source_user_id"] == 108)
        check(lineage["source_auth_version"] == 1 and lineage["native_before"]["auth_version"] == 1
              and lineage["explicit_user_auth_version"] == 2 and lineage["profile_policy_trigger_increment"] == 1
              and lineage["planned_final_auth_version"] == actual["auth_version"] == 3
              and actual["target_user_id"] == 109 and actual["target_personal_account_id"] == 110,
              "committed generation lineage lost its actual source/native owner binding")
        check(receipt["actual_identity_authority"] == self.query(M.identity_authority_sql(fresh)),
              "committed complete authority evidence differs from actual target readback")
        check(any(r["issuer"] == "https://accounts.google.com" and r["subject"] == "synthetic-google-sub-6"
                  and r["user_id"] == mapper[106]["target_user_id"] for r in full["oauth"]), "OAuth-only new owner binding missing")
        check(all(any(r["user_id"] == mapper[u]["target_user_id"] and r["group_name"] == "default" and r["enabled"]
                      for r in full["routing_grants"]) for u in (105, 106, 108)), "source-derived routing grant missing")
        self.completed += ["lost_real_COMMIT_ack_resolved_readonly", "exact_replay_no_writes_or_new_ids",
            "native_privileged_auth_version_and_activation_preserved", "history_identity_completed_without_duplicate_owner",
            "ordinary_status_auth_and_explicit_developer_facts_reconciled", "OAuth_subjects_keep_exact_owner_and_unique_authority",
            "committed_source_native_generation_lineage_contains_actual_readback",
            "source_authorized_active_native_routing_added", "unaffected_sessions_budgets_wallet_history_preserved_by_full_state_hash"]
        public = {"status": "MECHANICS_VERIFIED_RUNTIME_PENDING", "checks": self.completed,
                  "scope": M.SCOPE, "production_authorized": False, "production_access": False,
                  "real_google_login": "NOT_RUN", "password_runtime_login": "NOT_RUN",
                  "compiled_core78_manifest_sha256": M.U.CORE78_MANIFEST_SHA256}
        M.F.write_private(self.output, "result.public.json", M.canonical(public) + "\n")
        print(json.dumps(public))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--fixture", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args(argv)
    os.umask(0o077)
    regression = Regression(M.private_json(args.fixture), args.output)
    regression.run_identity()


if __name__ == "__main__":
    main()
