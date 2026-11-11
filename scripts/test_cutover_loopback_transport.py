"""Bound loopback targets use real restricted SCRAM sessions; sources stay Unix."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import unittest

import financial_import as F
from test_financial_import import material_fixture, source_fixture, snapshot_fixture

I = F.IDENTITY


class LoopbackContractTests(unittest.TestCase):
    def test_only_explicit_literal_loopback_and_private_passfile_are_accepted(self):
        with tempfile.TemporaryDirectory(dir=I.BACKUP_ROOT) as temporary:
            path = Path(temporary) / "pgpass"
            path.write_text("")
            path.chmod(0o600)
            connection = dict(transport="loopback-tcp", host="127.0.0.1", port=5432,
                database="synthetic", role="owner", login_role="installer", set_role="owner", pgpass_file=str(path))
            self.assertEqual(I.socket_connection(connection)["PGUSER"], "installer")
            self.assertEqual(I.socket_connection(connection)["PGREQUIREAUTH"], "scram-sha-256")
            self.assertNotIn("session_authorization", I.psql_options(connection, readonly=False))
            for change in ({"host": "localhost"}, {"host": "192.0.2.1"},
                           {"socket_directory": "/run/postgresql"}, {"transport": "tcp"}):
                with self.subTest(change=change), self.assertRaises(I.MappingError):
                    I.socket_connection({**connection, **change})

    def test_listener_addresses_and_privileged_sessions_are_not_laundered(self):
        identity = dict(tcp_connection=True, server_address="127.0.0.1", client_address="127.0.0.1",
                        listen_addresses="localhost", session_role_privileged=False, effective_role_privileged=False)
        self.assertTrue(I.local_target_transport(identity))
        self.assertTrue(I.local_target_transport({**identity, "listen_addresses": "127.0.0.1, ::1"}))
        for changes in ({"listen_addresses": "*"}, {"listen_addresses": "0.0.0.0"},
                        {"server_address": "192.0.2.1"}, {"client_address": None},
                        {"session_role_privileged": True}, {"effective_role_privileged": True}):
            with self.subTest(changes=changes):
                self.assertFalse(I.local_target_transport({**identity, **changes}))
        source = source_fixture()
        snapshot = snapshot_fixture(source)
        snapshot["catalog_fingerprint_sha256"] = I.current_contract()["fresh_catalog_sha256"]
        material = material_fixture(source, snapshot)
        material["target"]["instance"].update(identity)
        F.validate_material(material)
        material["target"]["instance"]["session_role_privileged"] = True
        with self.assertRaisesRegex(F.Error, "target_not_offline_owner"):
            F.validate_material(material)


@unittest.skipUnless(os.environ.get("LMM_CUTOVER_TCP_PG") == "1", "local SCRAM fixture opt-in")
class LoopbackSCRAMTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = Path(tempfile.mkdtemp(prefix="cutover-scram-pg-", dir=I.BACKUP_ROOT))
        cls.sock = cls.root / "sock"
        cls.sock.mkdir(mode=0o700)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            cls.port = probe.getsockname()[1]
        cls.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        cls.env.update(PGHOST=str(cls.sock), PGPORT=str(cls.port), PGDATABASE="postgres")
        cls.password = secrets.token_hex(24)
        cls.command(["initdb", "-D", str(cls.root / "pgdata"), "-A", "trust", "--no-locale"])
        hba = cls.root / "pgdata/pg_hba.conf"
        hba.write_text("host all all 127.0.0.1/32 scram-sha-256\nhost all all ::1/128 scram-sha-256\n" + hba.read_text())
        cls.command(["pg_ctl", "-D", str(cls.root / "pgdata"), "-l", str(cls.root / "postgres.log"), "-w", "start",
                     "-o", f"-k {cls.sock} -h localhost -p {cls.port}"])
        cls.addClassCleanup(cls.stop)
        cls.command(["psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1"],
                    sql="CREATE ROLE tcp_owner NOLOGIN; CREATE ROLE tcp_installer LOGIN PASSWORD '" +
                    cls.password + "'; GRANT tcp_owner TO tcp_installer; CREATE DATABASE tcp_target OWNER tcp_owner;")
        cls.passfile = cls.root / "pgpass"
        cls.passfile.write_text(f"127.0.0.1:{cls.port}:tcp_target:tcp_installer:{cls.password}\n")
        cls.passfile.chmod(0o600)
        cls.connection = dict(transport="loopback-tcp", host="127.0.0.1", port=cls.port,
            database="tcp_target", role="tcp_owner", login_role="tcp_installer", set_role="tcp_owner",
            pgpass_file=str(cls.passfile))
        print("SCRAM_PG_EVIDENCE=" + str(cls.root), flush=True)

    @classmethod
    def command(cls, argv, sql=None):
        result = subprocess.run(argv, input=sql, env=cls.env, text=True, capture_output=True)
        with (cls.root / "fixture.log").open("a") as log:
            log.write(result.stdout + result.stderr)
        if result.returncode:
            raise AssertionError(result.stderr[-1500:])
        return result

    @classmethod
    def stop(cls):
        cls.command(["pg_ctl", "-D", str(cls.root / "pgdata"), "-m", "fast", "-w", "stop"])

    def query(self, connection, sql, label):
        directory = self.root / label
        directory.mkdir(mode=0o700)
        return I.psql(connection, sql, readonly=True, private_dir=directory, label="actual")

    def test_real_scram_session_and_owner_transport_guard(self):
        actual = json.loads(self.query(self.connection, I.IDENTITY_SQL, "identity"))
        self.assertEqual(actual["session_user"], "tcp_installer")
        self.assertEqual(actual["role"], "tcp_owner")
        self.assertFalse(actual["session_role_privileged"])
        self.assertFalse(actual["effective_role_can_login"])
        self.assertTrue(actual["tcp_connection"])
        self.assertEqual(actual["server_address"], "127.0.0.1")
        I.verify_instance(actual, self.connection, actual, source=False)
        self.query(self.connection, "DO $$BEGIN IF " + I.target_transport_guard_sql(actual) +
                   " THEN RAISE EXCEPTION 'transport mismatch'; END IF; END$$;", "guard")
        with self.assertRaisesRegex(I.MappingError, "source_role_switch_forbidden"):
            I.verify_instance(actual, self.connection, actual, source=True)
        wrong = {**actual, "database_oid": int(actual["database_oid"]) + 1}
        with self.assertRaisesRegex(I.MappingError, "instance_receipt_mismatch"):
            I.verify_instance(actual, self.connection, wrong, source=False)

    def test_scram_wrong_password_is_rejected(self):
        bad = self.root / "bad-passfile"
        bad.write_text(f"127.0.0.1:{self.port}:tcp_target:tcp_installer:wrong\n")
        bad.chmod(0o600)
        with self.assertRaisesRegex(I.MappingError, "database_command_failed"):
            self.query({**self.connection, "pgpass_file": str(bad)}, I.IDENTITY_SQL, "wrong-password")

    def test_trust_hba_is_refused_even_with_private_password(self):
        hba = self.root / "pgdata/pg_hba.conf"
        original = hba.read_text()
        hba.write_text("host all all 127.0.0.1/32 trust\n" + original)
        try:
            self.command(["pg_ctl", "-D", str(self.root / "pgdata"), "reload"])
            # HBA reload is asynchronous. A connection authenticated using the
            # old SCRAM rule is harmless while the postmaster reloads its file.
            for attempt in range(50):
                label = "trust-refused-" + str(attempt)
                try:
                    self.query(self.connection, I.IDENTITY_SQL, label)
                except I.MappingError:
                    error = (self.root / label / "actual.stderr.private.txt").read_text()
                    self.assertIn("scram-sha-256", error)
                    break
                time.sleep(0.05)
            else:
                self.fail("libpq accepted trust authentication despite require_auth")
        finally:
            hba.write_text(original)
            self.command(["pg_ctl", "-D", str(self.root / "pgdata"), "reload"])


if __name__ == "__main__":
    unittest.main()
