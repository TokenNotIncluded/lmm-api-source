#!/usr/bin/env python3
"""Real PG regression against an explicitly prepared, disposable Core78 fixture.

No server installation, product compilation, source-branch edit or production
connection is performed here. The owner supplies a private fixture JSON:

  {"format_version":1,"scope":"current-runtime-keys-synthetic-fixture",
   "connection":{"transport":"unix","socket_directory":"/private/socket",
     "port":55474,"database":"lmm_runtime_keys_test_example",
     "role":"fixture_owner","login_role":"fixture_installer",
     "set_role":"fixture_owner","pgpass_file":"/private/pgpass"},
   "fixture_nonce":"<64 lowercase hex characters>",
   "core78_manifest":"/private/existing-core78-schema-manifest.json",
   "runtime_roles":["fixture_runtime"],"fixture_user_id":1,"fixture_group":"default"}

The database must already have the exact Core78 schema, a synthetic native
user with a granted active route, and database COMMENT
  lmm-runtime-keys-synthetic:<fixture_nonce>
The private Unix-only cluster has a migration instance nonce, NOLOGIN owner
and runtime, independent non-superuser installer, CONNECT fenced from others,
and no live service clients. Existing fixture money/history are retained.

Run (after the parent provides the isolated environment, never against prod):
  PYTHONPATH=scripts python scripts/test_migrate_current_runtime_keys_pg.py \
    --fixture /private/fixture.json --output /private/current-key-pg-results

This creates only synthetic Sessions/key metadata and budgets in that fixture.
A small Unix PostgreSQL protocol proxy really disconnects before COMMIT, then
drops the server's COMMIT CommandComplete on a separate committed attempt.
Those cases exercise rollback gaps and unknown-commit recovery without mocked
database outcomes. Normal/duplicate/CAS/concurrent cases use the same adapter.

The final private runtime-probes.json contains raw *synthetic* bearers and the
expected auth/quota behavior for the existing Core78 runtime harness. This
driver does not start a product server or claim an HTTP-auth/billing result.
Its public result explicitly keeps that required acceptance item NOT_RUN.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import threading
import time
import uuid

import migrate_current_runtime_keys as M
from test_migrate_current_runtime_keys import inputs, review, token


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def parse_single_result(raw):
    """psql -qAt emits native booleans as t/f, JSON booleans as true/false."""
    values = [line for line in raw.splitlines() if line.strip()]
    check(len(values) == 1, "expected one private SQL result")
    if values[0] in ("t", "f"):
        return values[0] == "t"
    return json.loads(values[0])


def read_exact(sock, length):
    data = bytearray()
    while len(data) < length:
        part = sock.recv(length - len(data))
        if not part:
            raise EOFError
        data.extend(part)
    return bytes(data)


class CommitDropProxy:
    """Local synthetic-test-only PG v3 relay, not a mock or production feature."""
    def __init__(self, socket_path, directory, port, *, before_commit):
        self.socket_path = str(socket_path)
        self.directory = directory
        self.port = port
        self.before_commit = before_commit
        self.triggered = threading.Event()
        self.done = threading.Event()
        self.errors = []
        self.clients = []

    def __enter__(self):
        self.directory.mkdir(mode=0o700)
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(self.directory / (".s.PGSQL." + str(self.port))))
        self.listener.listen(1)
        self.listener.settimeout(10)
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()
        return self

    def close_connections(self):
        for conn in self.clients:
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                conn.close()
            except OSError:
                pass

    def _serve(self):
        try:
            client, _ = self.listener.accept()
            upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            upstream.connect(self.socket_path)
            self.clients = [client, upstream]
            # The dedicated Unix socket carries no network traffic. Decline
            # optional SSL/GSS negotiation, then relay real PG authentication.
            for _ in range(3):
                header = read_exact(client, 4)
                length = struct.unpack("!I", header)[0]
                check(8 <= length <= 65536, "unexpected PG startup frame")
                payload = read_exact(client, length - 4)
                if length == 8 and struct.unpack("!I", payload)[0] in (80877103, 80877104):
                    client.sendall(b"N")
                    continue
                upstream.sendall(header + payload)
                break
            else:
                raise AssertionError("no PG startup frame after negotiation")
            front = threading.Thread(target=self._front, args=(client, upstream), daemon=True)
            front.start()
            while True:
                header = read_exact(upstream, 5)
                length = struct.unpack("!I", header[1:])[0]
                check(4 <= length <= 256 * 1024 * 1024, "unexpected PG backend frame")
                payload = read_exact(upstream, length - 4)
                if not self.before_commit and header[:1] == b"C" and payload == b"COMMIT\0":
                    # PostgreSQL has durably committed; the caller never gets
                    # its confirmation or the post-COMMIT result JSON.
                    self.triggered.set()
                    self.close_connections()
                    break
                client.sendall(header + payload)
            front.join(timeout=5)
        except (EOFError, OSError):
            pass
        except BaseException as exc:
            self.errors.append(type(exc).__name__)
        finally:
            self.close_connections()
            self.done.set()

    def _front(self, client, upstream):
        try:
            while True:
                header = read_exact(client, 5)
                length = struct.unpack("!I", header[1:])[0]
                check(4 <= length <= 256 * 1024 * 1024, "unexpected PG frontend frame")
                payload = read_exact(client, length - 4)
                if self.before_commit and header[:1] == b"Q" and payload.rstrip(b"\0; \r\n").upper() == b"COMMIT":
                    # All INSERTs ran; the server has not received COMMIT.
                    self.triggered.set()
                    self.close_connections()
                    return
                upstream.sendall(header + payload)
        except (EOFError, OSError):
            pass
        except BaseException as exc:
            self.errors.append(type(exc).__name__)

    def __exit__(self, *_args):
        self.close_connections()
        self.listener.close()
        self.thread.join(timeout=5)
        check(self.triggered.is_set() and not self.errors, "proxy did not observe the requested real COMMIT boundary")


class Regression:
    def __init__(self, fixture, output):
        self.fixture = fixture
        self.output = output
        self.counter = 0
        self.completed = []
        self.connection = fixture["connection"]
        self.nonce = fixture["fixture_nonce"]
        M.need(fixture.get("format_version") == 1 and fixture.get("scope") == "current-runtime-keys-synthetic-fixture",
               "current_keys_fixture_contract_invalid")
        M.need(self.connection.get("transport", "unix") == "unix", "current_keys_test_unix_only")
        M.need(M.IDENTIFIER.fullmatch(self.connection["database"]) is not None
               and self.connection["database"].startswith("lmm_runtime_keys_test_"),
               "current_keys_synthetic_database_required")
        M.need(type(self.nonce) is str and M.HEX.fullmatch(self.nonce), "current_keys_fixture_nonce_invalid")
        self.contract = M.core78_contract(M.C.private_bytes(Path(fixture["core78_manifest"])))
        output.mkdir(mode=0o700, parents=True, exist_ok=False)
        M.F.write_private(output, "fixture.private.json", M.canonical(fixture) + "\n")
        self.query("SELECT CASE WHEN shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),"
                   "'pg_database')=" + M.literal("lmm-runtime-keys-synthetic:" + self.nonce) +
                   " THEN true ELSE false END", expect=True)
        self.inventory = self.query(M.F.inventory_sql())
        self.fixture_user_id = fixture["fixture_user_id"]
        M.need(type(self.fixture_user_id) is int and self.fixture_user_id > 0, "current_keys_fixture_user_invalid")
        self.users = self.query(M.C.native_users_sql())
        self.owner = next(r for r in self.users if r["user_id"] == self.fixture_user_id)
        self.group = fixture["fixture_group"]
        check(self.owner["active"] and self.owner["developer_access_granted"]
              and self.group in self.owner["routing_groups"], "fixture user needs existing full native route authority")
        # Exact contract/instance checks before seeding anything, including IDs.
        initial = self.query(M.snapshot_sql(self.contract, self.inventory, "synthetic-initial-check"))
        M.validate_snapshot(initial, self.contract)
        check_material = {"snapshot": initial, "contract": self.contract, "before_state": initial["state"],
                          "runtime_roles": fixture["runtime_roles"]}
        self.run_sql("DO $fixture$ BEGIN " + M._test_fixture_guard(check_material, self.nonce) +
                     M._guard_sql(check_material) + " END $fixture$;")
        self.fixture_snapshot = initial

    def run_sql(self, sql, *, expect_ok=True, connection=None, app=None):
        self.counter += 1
        label = "case-" + str(self.counter)
        M.F.write_private(self.output, label + ".sql", sql)
        code, raw = M.F.run_sql(connection or self.connection, sql, self.output,
            app or ("lmm-current-keys-" + uuid.uuid4().hex), label=label)
        if expect_ok:
            check(code == 0, "psql failed; inspect private evidence " + label)
        return code, raw, label

    def query(self, sql, *, expect=None):
        _, raw, _ = self.run_sql(sql)
        value = parse_single_result(raw)
        if expect is not None:
            check(value == expect, "fixture guard failed")
        return value

    def state(self, migration="synthetic-state-check"):
        return self.query(M.state_sql(self.inventory, migration))

    def seed_sessions_and_budget(self):
        uid, aid, version = self.owner["user_id"], self.owner["personal_account_id"], self.owner["auth_version"]
        self.session_bearers = ["synthetic-session-" + uuid.uuid4().hex for _ in range(4)]
        inserted = []
        for n, bearer in enumerate(self.session_bearers):
            expiry = "clock_timestamp()+interval '1 day'" if n == 0 else "to_timestamp(300)"
            inserted.append("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) "
                f"VALUES(decode('{hashlib.sha256(bearer.encode()).hexdigest()}','hex'),'session',{uid},{version},{aid},{expiry}) RETURNING id")
        self.session_ids = []
        for statement in inserted:
            cid = self.query(statement)
            self.session_ids.append(cid)
            # Core78 authenticates Sessions through their append-only native
            # assurance as well as the credential digest. A bare row is not a
            # valid Session fixture (identity::mfa::from_session_in_mode).
            self.run_sql("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,"
                "factor_revision,factor_level,method,authenticated_at) "
                f"VALUES({cid},{uid},{version},0,1,'primary',clock_timestamp())")
        # A native budget unrelated to the incoming keys must survive exactly.
        self.native_budget_id = self.query("INSERT INTO core_charging.budgets(scope,account_id,issuer_user_id,period,limit_credits,seconds) "
            f"VALUES('account',{aid},{uid},'custom',9999,12345) ON CONFLICT DO NOTHING RETURNING id")
        # A reusable fixture must not already use this synthetic scope identity.
        check(type(self.native_budget_id) is int, "fixture budget identity already in use")
        self.completed.append("seeded_valid_and_expired_sessions_and_native_budget")

    def source_target(self, *, ids=None, archive=None):
        s, t = inputs()
        source_uid = 37
        current_ids = self.session_ids if ids is None else ids
        raw = [token(id=tid, key="SyntheticWireKey" + uuid.uuid4().hex + str(n),
                user_id=source_uid, group="" if self.group == "default" else self.group, one_time_reveal=(n != 0),
                status=2 if n == 1 else 3 if n == 2 else 1,
                expired_time=500 if n == 2 else -1,
                remain_quota=-15 if n == 3 else 30, unlimited_quota=(n == 3))
               for n, tid in enumerate(current_ids)]
        s["tokens"] = raw
        s["users"][0].update(auth_version=self.owner["auth_version"], group=self.group)
        s["policy"].update(auto_groups=[self.group])
        s["policy"]["group_authority"] = {"user_usable_groups": {self.group: self.group},
            "special_usable_groups": {}, "group_ratios": {self.group: 1}, "group_group_ratios": {}}
        if archive:
            s["source_archive_sha256"] = archive
        t["source_archive_sha256"] = s["source_archive_sha256"]
        mapping = [{"source_user_id": source_uid, "target_user_id": self.owner["user_id"],
                    "target_personal_account_id": self.owner["personal_account_id"]}]
        t["source_target_id_map"] = mapping
        t["source_target_id_map_sha256"] = hashlib.sha256(M.P.id_map_bytes(mapping).encode()).hexdigest()
        t["users"] = self.query(M.C.native_users_sql())
        return s, t

    def prepare(self, source, target, migration, *, hide_receipt_for_adversarial_case=False):
        snap = self.query(M.snapshot_sql(self.contract, self.inventory, migration))
        if hide_receipt_for_adversarial_case:
            # Exercise the independent database namespace guard after a forged
            # caller omits known receipts from the Python projection.
            snap["key_imports"] = []
        m = M.prepare_mechanics(source, target, review(source, target), snapshot=snap,
            contract=self.contract, migration_id=migration, source_namespace={"system_identifier": "999000111222",
                "database": "synthetic_legacy", "schema": "synthetic_source"},
            runtime_roles=self.fixture["runtime_roles"], encryption_key=bytes(range(32)))
        M.F.write_private(self.output, migration + ".material.private.json", M.canonical(m) + "\n")
        return m

    def reject(self, sql, error):
        before = self.state()
        code, _, label = self.run_sql(sql, expect_ok=False)
        check(code != 0, "expected fail-closed SQL")
        stderr = M.C.private_bytes(self.output / (label + ".stderr.private.txt")).decode()
        check(error in stderr, "unexpected SQL refusal: inspect " + label)
        check(self.state() == before, "rejected preview changed rows or consumed sequence values")

    def proxy_attempt(self, m, *, before_commit):
        conn = deepcopy(self.connection)
        path = Path(conn["socket_directory"]) / (".s.PGSQL." + str(conn["port"]))
        directory = self.output / ("proxy-" + uuid.uuid4().hex[:8])
        app = "lmm-current-keys-" + uuid.uuid4().hex
        with CommitDropProxy(path, directory, conn["port"], before_commit=before_commit):
            conn["socket_directory"] = str(directory)
            code, _, _ = self.run_sql(M.build_synthetic_transaction_sql(m, self.nonce),
                                     connection=conn, app=app, expect_ok=False)
            check(code != 0, "dropped commit unexpectedly acknowledged")
        for _ in range(50):
            active = self.query("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=" + M.literal(app) + ")")
            if not active:
                break
            time.sleep(0.1)
        check(not active, "original attempt remains active; do not retry")
        return self.query(M.build_resolve_sql(m, app))

    def concurrent_fence(self, m):
        env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        env.update(M.I.socket_connection(self.connection))
        app = "synthetic-fence-" + uuid.uuid4().hex
        env.update(PGAPPNAME=app, PGOPTIONS=M.I.psql_options(self.connection, readonly=False))
        proc = subprocess.Popen(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True, env=env)
        try:
            proc.stdin.write(f"SELECT pg_advisory_lock({M.FENCE}); SELECT pg_sleep(20);\n")
            proc.stdin.close()
            ready = False
            for _ in range(50):
                ready = self.query("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=" +
                    M.literal(app) + " AND wait_event='PgSleep')")
                if ready:
                    break
                time.sleep(0.1)
            check(ready, "competing transaction did not obtain fence")
            self.reject(M.build_synthetic_transaction_sql(m, self.nonce), "current_keys_import_busy")
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
        self.completed.append("concurrent_writer_fails_before_allocation")

    def run(self):
        self.seed_sessions_and_budget()
        source, target = self.source_target()
        migration = "synthetic-used78-" + uuid.uuid4().hex[:12]
        original = self.prepare(source, target, migration)
        sql = M.build_synthetic_transaction_sql(original, self.nonce)
        self.concurrent_fence(original)

        # A pre-existing budget CAS drift must be noticed before nextval.
        self.run_sql(f"UPDATE core_charging.budgets SET limit_credits=10000 WHERE id={self.native_budget_id}")
        self.reject(sql, "current_keys_target_state_changed")
        self.run_sql(f"UPDATE core_charging.budgets SET limit_credits=9999 WHERE id={self.native_budget_id}")
        self.completed.append("preexisting_budget_drift_rejected_before_nextval")

        # Genuine socket loss before COMMIT rolls rows back but leaves normal
        # sequence gaps. The read-only resolver must not call absence rollback.
        before = self.state(migration)
        resolved = self.proxy_attempt(original, before_commit=True)
        check(resolved["status"] == "UNKNOWN" and resolved["automatic_retry_allowed"] is False,
              "receipt absence was incorrectly promoted to confirmed rollback")
        after = self.state(migration)
        check(after["tables"] == before["tables"], "disconnected transaction changed existing rows")
        for seq in M.SEQUENCES:
            check(after["sequences"][seq]["last_value"] > before["sequences"][seq]["last_value"],
                  "expected normal non-rollbackable sequence advancement")
        self.reject(sql, "current_keys_target_state_changed")
        self.completed.append("disconnect_rolls_back_rows_and_preserves_monotone_sequence_gaps")

        # The test proxy proves COMMIT was never sent. A deliberately new
        # preview now binds the advanced sequence state; no automatic retry.
        m = self.prepare(source, target, migration)
        check(m["records"][0]["target_credential_id"] > original["records"][-1]["target_credential_id"],
              "new preview reused an allocated ID")
        result = self.query(M.build_synthetic_transaction_sql(m, self.nonce))
        check(result["status"] == "COMMITTED", "append did not commit")
        state = self.state(migration)
        check(self.query(M.build_synthetic_transaction_sql(m, self.nonce)) == result, "exact replay differs")
        check(self.state(migration) == state, "exact replay changed rows or advanced a sequence")
        preserved = self.query(M.state_sql(self.inventory, migration, inserted=m))
        check(preserved["tables"] == m["before_state"]["tables"], "original Sessions/budgets/funds/history changed")
        check(set(self.session_ids).isdisjoint({r["target_credential_id"] for r in m["records"]}), "source ID overlap overwrote Sessions")
        self.completed.extend(["existing_sessions_budgets_funds_history_preserved", "source_id_collision_remapped",
                               "exact_replay_no_new_ids", "normal_sequence_allocation"])

        # The SQL-level namespace/key guard rejects a different archive and
        # batch even when an adversarial caller hides the first receipt.
        dupe_source, dupe_target = self.source_target(archive="f" * 64)
        dupe = self.prepare(dupe_source, dupe_target, migration + "-dupe", hide_receipt_for_adversarial_case=True)
        self.reject(M.build_synthetic_transaction_sql(dupe, self.nonce), "current_keys_source_identity_already_imported")
        self.completed.append("source_site_plus_key_dedup_across_batches_and_archives")

        # New keys may share their visible source IDs only at another source
        # site; this case instead keeps the site and supplies distinct IDs.
        more_source, more_target = self.source_target(ids=[i + 100000 for i in self.session_ids])
        more = self.prepare(more_source, more_target, migration + "-ack")
        resolved = self.proxy_attempt(more, before_commit=False)
        check(resolved["status"] == "COMMITTED" and resolved["automatic_retry_allowed"] is False,
              "real lost COMMIT confirmation did not resolve from immutable receipt")
        committed = self.state(more["migration_id"])
        self.query(M.build_resolve_sql(more, "lmm-current-keys-" + uuid.uuid4().hex))
        check(self.state(more["migration_id"]) == committed, "read-only resolution mutated target")
        self.completed.append("lost_server_commit_confirmation_resolves_without_reapplication")

        keys = []
        for raw, mapped in zip(source["tokens"], m["records"]):
            keys.append({"bearer": raw["key"], "credential_id": mapped["target_credential_id"],
                "expected_auth": "disabled" if raw["status"] == 2 else "expired" if raw["status"] == 3 else "allowed",
                "used_units": raw["used_quota"], "remaining_units": raw["remain_quota"],
                "unlimited": raw["unlimited_quota"], "recoverable": not raw["one_time_reveal"]})
        probes = {"format_version": 1, "scope": "current-runtime-key-synthetic-runtime-probes",
            "database": self.connection["database"], "fixture_nonce": self.nonce,
            "expected_core78_manifest_sha256": M.CORE78_MANIFEST_SHA256,
            "session": {"bearer": self.session_bearers[0], "credential_id": self.session_ids[0]},
            "keys": keys, "user_id": self.owner["user_id"], "account_id": self.owner["personal_account_id"],
            "encryption_key_hex": bytes(range(32)).hex(),
            "required_runtime_assertions": ["old valid Session still authenticates; expired Sessions stay expired",
                "new allowed/disabled/expired key authentication matches original status",
                "recoverable key decrypts using the new ID; one-time key has no recovery copy",
                "historical used=70 and remaining=30 admit at most 30 additional units",
                "unlimited key retains signed displayed remainder -15 without imposing a finite cap",
                "duplicate migration and read-only resolution do not increase wallet or key quota"]}
        M.F.write_private(self.output, "runtime-probes.private.json", M.canonical(probes) + "\n")
        public = {"status": "MECHANICS_VERIFIED_RUNTIME_PENDING", "checks": self.completed,
            "runtime_authentication_and_quota": "NOT_RUN", "production_authorized": False,
            "production_blocker": M.BLOCKER, "product_rebuilt": False}
        M.F.write_private(self.output, "result.public.json", M.canonical(public) + "\n")
        print(json.dumps(public))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    Regression(M.C.load_json(args.fixture), args.output).run()


if __name__ == "__main__":
    main()
