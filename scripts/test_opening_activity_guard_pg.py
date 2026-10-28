"""Real PG18 regression; always creates its own Unix-socket-only cluster.

Run: python3 -B scripts/test_opening_activity_guard_pg.py -v
Successful fixtures are stopped and removed. Failures retain the cluster,
generated SQL, command output and server log under the printed /tmp path.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import tempfile
import time
import traceback
import unittest

import legacy_financial_opening as opening


def guard_do(*, final=False):
    return (
        "DO $opening_activity_guard$\nBEGIN\n"
        + opening.other_clients_guard_sql(final=final)
        + "END;\n$opening_activity_guard$;\n"
    )


class PsqlSession:
    def __init__(self, fixture, name):
        self.sql_path = fixture.root / (name + ".sql")
        self.log_path = fixture.root / (name + ".log")
        self.pending = b""
        self.step = 0
        self.process = subprocess.Popen(
            [
                fixture.tools["psql"], "-X", "-w", "-qAt",
                "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose",
                "-h", str(fixture.socket), "-p", "65432",
                "-U", "opening_guard_test", "-d", "postgres",
            ],
            env=fixture.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, bufsize=0,
        )

    def record(self, path, data):
        with path.open("ab") as output:
            output.write(data)

    def exchange(self, sql):
        self.step += 1
        marker = f"opening_guard_step_{self.step}".encode()
        command = sql.encode() + b"\n\\echo " + marker + b"\n"
        self.record(self.sql_path, command)
        self.process.stdin.write(command)
        deadline = time.monotonic() + 15
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            while marker + b"\n" not in self.pending:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise AssertionError(f"psql barrier timed out; see {self.log_path}")
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise AssertionError(f"psql exited before barrier; see {self.log_path}")
                self.record(self.log_path, chunk)
                self.pending += chunk
        output, self.pending = self.pending.split(marker + b"\n", 1)
        return output.decode().strip()

    def finish(self, sql="\\q\n"):
        command = sql.encode()
        self.record(self.sql_path, command)
        output, _ = self.process.communicate(command, timeout=15)
        self.record(self.log_path, output)
        return self.process.returncode, (self.pending + output).decode()


class PrivatePostgres:
    def __init__(self):
        # PG service/options variables must never redirect a test connection.
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith("PG")}
        self.env.update(LC_ALL="C", PSQL_HISTORY="/dev/null")
        self.tools = {}
        self.versions = {}
        for name in ("initdb", "pg_ctl", "psql", "postgres"):
            executable = shutil.which(name)
            if executable is None:
                raise AssertionError(f"required PostgreSQL 18 tool missing: {name}")
            version = subprocess.run(
                [executable, "--version"], env=self.env, check=True,
                capture_output=True, text=True, timeout=10,
            ).stdout.strip()
            if "(PostgreSQL) 18." not in version:
                raise AssertionError(f"PostgreSQL 18 required: {version}")
            self.tools[name] = executable
            self.versions[name] = version
        if shutil.disk_usage("/tmp").free < 256 * 1024 * 1024:
            raise AssertionError("private PostgreSQL fixture needs 256 MiB free in /tmp")
        self.root = Path(tempfile.mkdtemp(prefix="opening-activity-guard-pg-", dir="/tmp"))
        self.data = self.root / "data"
        self.socket = self.root / "socket"
        self.socket.mkdir(mode=0o700)
        metadata = {
            "tools": self.tools,
            "versions": self.versions,
            "sha256": {
                path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                for path in (Path(__file__).resolve(), Path(opening.__file__).resolve())
            },
        }
        (self.root / "fixture.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(f"\nPrivate fixture: {self.root}", flush=True)
        print(self.versions["postgres"], flush=True)

    def run(self, label, command):
        with (self.root / "commands.log").open("a") as log:
            log.write(json.dumps(command) + "\n")
        with (self.root / (label + ".log")).open("wb") as log:
            subprocess.run(
                command, env=self.env, stdout=log, stderr=subprocess.STDOUT,
                check=True, timeout=45,
            )

    def start(self):
        self.run("initdb", [
            self.tools["initdb"], "-D", str(self.data), "--no-sync",
            "--no-locale", "--encoding=UTF8", "--username=opening_guard_test",
            "--auth-local=trust", "--auth-host=reject",
        ])
        with (self.data / "postgresql.conf").open("a") as config:
            config.write(
                "\nlisten_addresses = ''\n"
                f"unix_socket_directories = '{self.socket}'\n"
                "unix_socket_permissions = 0700\n"
                "port = 65432\n"
                "autovacuum = off\n"
                "max_connections = 8\n"
                "shared_buffers = '16MB'\n"
                "log_statement = 'all'\n"
                "log_connections = on\n"
                "log_disconnections = on\n"
            )
        self.run("pg_ctl-start", [
            self.tools["pg_ctl"], "-D", str(self.data),
            "-l", str(self.root / "postgres.log"), "-w", "-t", "20", "start",
        ])

    def clean_success(self):
        self.run("pg_ctl-stop", [
            self.tools["pg_ctl"], "-D", str(self.data),
            "-w", "-t", "20", "-m", "fast", "stop",
        ])
        shutil.rmtree(self.root)
        print(f"Stopped and removed successful fixture: {self.root}", flush=True)


class OpeningActivityGuardPostgresTests(unittest.TestCase):
    def test_final_guard_refreshes_initial_activity_snapshot(self):
        fixture = PrivatePostgres()
        try:
            fixture.start()
            primary = PsqlSession(fixture, "primary")
            identity = json.loads(primary.exchange("""
SELECT json_build_object(
 'version', current_setting('server_version_num')::int,
 'data', current_setting('data_directory'),
 'listen', current_setting('listen_addresses'),
 'socket', current_setting('unix_socket_directories'),
 'address', inet_server_addr());
"""))
            self.assertEqual(identity["version"] // 10000, 18)
            self.assertEqual(identity["data"], str(fixture.data))
            self.assertEqual(identity["listen"], "")
            self.assertEqual(identity["socket"], str(fixture.socket))
            self.assertIsNone(identity["address"])
            initial = guard_do()
            final = guard_do(final=True)

            # Both production fragments must also accept an uncontended transaction.
            self.assertEqual(primary.exchange(
                "BEGIN ISOLATION LEVEL SERIALIZABLE;\n"
                "SET LOCAL stats_fetch_consistency = 'snapshot';\n"
                + initial + final
                + "SELECT 'control_accepted';\nROLLBACK;\n"
            ), "control_accepted")
            print("PASS: generated initial and final DO guards accept no other client",
                  flush=True)

            transaction_state = """
SELECT json_build_object(
 'pid', pg_backend_pid(), 'xid', pg_current_xact_id()::text,
 'other_clients', (SELECT count(*) FROM pg_stat_activity
   WHERE datid = (SELECT oid FROM pg_database WHERE datname = current_database())
     AND pid <> pg_backend_pid()));
"""
            before = json.loads(primary.exchange(
                "BEGIN ISOLATION LEVEL SERIALIZABLE;\n"
                "SET LOCAL stats_fetch_consistency = 'snapshot';\n"
                + initial + transaction_state
            ))
            self.assertEqual(before["other_clients"], 0)

            contender = PsqlSession(fixture, "contender")
            contender_pid = int(contender.exchange("SELECT pg_backend_pid();"))
            self.assertNotEqual(contender_pid, before["pid"])
            self.assertIsNone(contender.process.poll())

            # Do not clear stats here: prove the original transaction is still
            # using its stale activity snapshot before the final production guard.
            stale = json.loads(primary.exchange(transaction_state))
            self.assertEqual(stale, before)
            print(f"PASS: same backend {before['pid']}, transaction {before['xid']}; "
                  f"stale snapshot hides connected backend {contender_pid}", flush=True)

            status, output = primary.finish(final + "SELECT 'unexpected_acceptance';\n")
            self.assertEqual(status, 3, output)
            self.assertIn(
                "P0001: opening final catalog or client mismatch", output,
            )
            self.assertNotIn("unexpected_acceptance", output)
            self.assertIsNone(contender.process.poll())
            print("PASS: generated final DO guard rejects the new client "
                  "(psql exit 3, SQLSTATE P0001)", flush=True)
            status, output = contender.finish()
            self.assertEqual(status, 0, output)
            fixture.clean_success()
        except BaseException:
            (fixture.root / "failure.txt").write_text(traceback.format_exc())
            print(f"FAILURE EVIDENCE RETAINED; cluster not stopped: {fixture.root}",
                  flush=True)
            raise


if __name__ == "__main__":
    unittest.main()
