#!/usr/bin/env python3
"""Exercise actual offline CLI transactions against disposable PostgreSQL 18.

Development verification, not production cutover or whole-site acceptance.
All clusters, units, credentials and faults belong to this invocation.
"""
import argparse
import copy
import fcntl
import hashlib
import http.client
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time


def sha(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def need(ok, message):
    if not ok:
        raise AssertionError(message)


def framed(rows):
    h = hashlib.sha256()
    for row in rows:
        value = row.encode()
        h.update(len(value).to_bytes(8, "big"))
        h.update(value)
    return h.hexdigest()


class Fixture:
    def __init__(self, args):
        self.args = args
        self.root = args.output.resolve()
        self.root.mkdir(mode=0o700)
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(("PG", "LMM_", "PAYMENT_", "SQLX_"))
                    and k not in ("DATABASE_URL", "SQL_DSN")}
        self.env.update(XDG_RUNTIME_DIR=f"/run/user/{os.getuid()}",
                        DBUS_SESSION_BUS_ADDRESS=f"unix:path=/run/user/{os.getuid()}/bus")
        self.events, self.clusters, self.proxies = [], [], []
        self.unit, self.unit_file = None, None
        self.secrets = []
        module = args.source / "tests/console/independent_process/pg_fault.py"
        spec = importlib.util.spec_from_file_location("offline_pg_fault", module)
        self.fault = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.fault)

    def save(self, name, value):
        (self.root / name).write_text(json.dumps(value, indent=2) + "\n")

    def file(self, name, value):
        p = self.root / name
        p.write_text(value)
        p.chmod(0o600)
        return p

    def run(self, name, argv, body=None, expected=0, env=None):
        n = len(self.events)
        result = subprocess.run([str(a) for a in argv], input=body, capture_output=True,
                                env=env or self.env, timeout=180)
        self.file(f"{n:03d}-{name}.log", (result.stdout + result.stderr).decode(errors="replace"))
        self.events.append({"name": name, "argv": [str(a) for a in argv],
                            "input_sha256": hashlib.sha256(body).hexdigest() if body else None,
                            "exit": result.returncode})
        self.save("commands.json", self.events)
        if expected is not None:
            need(result.returncode == expected, name + "_exit")
        return result

    def sql(self, c, sql, database="postgres", user="postgres", owner=False):
        env = dict(self.env)
        if user == "lmm_core_installer":
            env["PGPASSWORD"] = self.installer_password
        if user == "lmm_core_runtime" and hasattr(self, "runtime_password"):
            env["PGPASSWORD"] = self.runtime_password
        if owner:
            env["PGOPTIONS"] = "-c role=lmm_core_owner"
        r = self.run("sql", ["psql", "-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1",
                             "-h", c["socket"], "-p", c["port"], "-U", user, "-d", database],
                     sql.encode(), env=env)
        return r.stdout.decode().strip()

    def cluster(self, label):
        d = self.root / label
        d.mkdir(mode=0o700)
        c = {"directory": d, "data": d / "data", "socket": Path(tempfile.mkdtemp(prefix="lmm-off-", dir="/tmp")),
             "port": 55601, "started": False, "hba": d / "external-hba.conf"}
        c["hba"].write_text("local all all trust\n")
        c["hba"].chmod(0o600)
        self.clusters.append(c)
        self.run("initdb", ["initdb", "-D", c["data"], "-U", "postgres", "--encoding=UTF8", "--no-locale", "--auth=trust"])
        self.start(c)
        return c

    def start(self, c):
        options = (f"-c listen_addresses= -c unix_socket_directories={c['socket']} -p {c['port']} "
                   f"-c hba_file={c['hba']} -c shared_buffers=16MB -c max_connections=20")
        self.run("pg-start", ["pg_ctl", "-D", c["data"], "-l", c["directory"] / "postgres.log", "-o", options, "-w", "start"])
        c["started"] = True

    def stop(self, c):
        if c["started"]:
            self.run("pg-stop", ["pg_ctl", "-D", c["data"], "-m", "fast", "-w", "stop"])
            c["started"] = False

    def url(self, c, user, database, password="", options=""):
        auth = user + (":" + password if password else "")
        return f"postgresql://{auth}@localhost/{database}?host={c['socket']}&port={c['port']}&sslmode=disable{options}"

    def config(self, name, url):
        path = self.file(name + ".url", url)
        self.secrets.append(path)
        return self.file(name + ".toml", f"version=1\n[database]\nurl_file={json.dumps(str(path))}\n")

    def cli(self, name, config, request, expected=0):
        p = self.run(name, [self.args.core, "--config", config, "upgrade-offline"],
                     json.dumps(request).encode(), expected=expected)
        return p, json.loads(p.stdout)

    def target_sql(self, sql):
        self.sql(self.target, "ALTER DATABASE coretarget ALLOW_CONNECTIONS true")
        try:
            return self.sql(self.target, sql, "coretarget", "lmm_core_installer", True)
        finally:
            self.sql(self.target, "ALTER DATABASE coretarget ALLOW_CONNECTIONS false")

    def assert_closed(self):
        need(self.sql(self.target, "SELECT NOT datallowconn FROM pg_database WHERE datname='coretarget'") == "t", "target_not_closed")
        state = self.run("unit-state", ["systemctl", "--user", "show", self.unit, "--property=MainPID,UnitFileState,ActiveState"]).stdout.decode()
        need("MainPID=0" in state and "UnitFileState=masked" in state, "unit_not_masked")
        need(self.target["hba"].read_text().startswith("# lmm offline v3 "), "external_hba_not_retained")

    def setup(self):
        self.target = self.cluster("target")
        self.sql(self.target, "CREATE DATABASE coretarget")
        self.run("canonical-roles", ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-h", self.target["socket"],
                 "-p", self.target["port"], "-U", "postgres", "-d", "coretarget", "-f",
                 self.args.source / "deployment/postgres/create-core-roles.sql"])
        self.sql(self.target, "ALTER DATABASE coretarget OWNER TO lmm_core_owner")
        self.installer_password = secrets.token_hex(24)
        self.sql(self.target, f"SET password_encryption='scram-sha-256'; CREATE ROLE lmm_core_installer LOGIN NOINHERIT PASSWORD '{self.installer_password}'; "
                 "GRANT lmm_core_owner TO lmm_core_installer WITH INHERIT FALSE, SET TRUE; GRANT CONNECT ON DATABASE coretarget TO lmm_core_installer; CREATE ROLE lmm_extensions LOGIN; "
                 "REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner", "coretarget")
        owner = self.config("owner", self.url(self.target, "lmm_core_installer", "coretarget", self.installer_password, "&options=-c%20role%3Dlmm_core_owner"))
        self.run("previous-native-init", [self.args.previous_core, "--config", owner, "init-db"])
        self.owner_password = "fixture-" + secrets.token_hex(16)
        receipt = json.loads(self.run("previous-native-first-owner", [self.args.previous_core, "--config", owner, "create-first-owner"],
                  json.dumps({"username": "offlineowner", "display_name": "Synthetic owner", "password": self.owner_password}).encode()).stdout)
        need(receipt.get("setup_completed") is True, "first_owner_setup")
        self.run("canonical-grants", ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-h", self.target["socket"],
                 "-p", self.target["port"], "-U", "postgres", "-d", "coretarget", "-v", "enable_console=true",
                 *(["-v", "enable_web_identity=true"] if getattr(self.args, "verify_runtime", False) else []), "-f",
                 self.args.source / "deployment/postgres/grant-core-runtime.sql"])
        self.sql(self.target, "UPDATE core_identity.user_profiles SET email='legacy@example.invalid',email_login_allowed=true,setting='{\"offline\":true}'; "
                 "INSERT INTO core_console.options(key,value) VALUES('Notice','kept before upgrade') ON CONFLICT(key) DO UPDATE SET value=excluded.value", "coretarget")
        self.facts_sql = "SELECT jsonb_build_object('users',(SELECT jsonb_agg(to_jsonb(u) ORDER BY id) FROM core_identity.users u),'profiles',(SELECT jsonb_agg(to_jsonb(p) ORDER BY user_id) FROM core_identity.user_profiles p),'notice',(SELECT value FROM core_console.options WHERE key='Notice'))::text"
        self.before = self.sql(self.target, self.facts_sql, "coretarget")
        self.sql(self.target, "CREATE DATABASE freshcheck")
        self.sql(self.target, "ALTER DATABASE freshcheck OWNER TO lmm_core_owner")
        self.sql(self.target, "GRANT USAGE,CREATE ON SCHEMA public TO lmm_core_owner", "freshcheck")
        fresh = self.config("fresh", self.url(self.target, "postgres", "freshcheck", options="&options=-c%20role%3Dlmm_core_owner"))
        self.run("new-native-fresh-init", [self.args.core, "--config", fresh, "init-db"])
        manifest = json.loads(subprocess.check_output([str(self.args.core), "schema-manifest"], env=self.env))
        actual = self.sql(self.target, manifest["catalog"]["query_utf8"], "freshcheck").splitlines()
        expected = json.loads((self.args.reference).read_text())["target_catalog_sha256"]
        need(framed(actual) == expected, "fresh_native_catalog_not_reference")
        self.save("fresh-catalog.json", {"actual": framed(actual), "reference": expected, "definitions": len(actual)})
        self.stop(self.target)
        self.backup = self.target["directory"] / "pre-upgrade"
        shutil.copytree(self.target["data"], self.backup)
        self.start(self.target)
        runtime_url = self.file("runtime.url", self.url(self.target, "lmm_core_runtime", "coretarget"))
        self.secrets.append(runtime_url)
        port_socket = socket.socket()
        port_socket.bind(("127.0.0.1", 0))
        self.http_port = port_socket.getsockname()[1]
        port_socket.close()
        runtime = self.file("runtime.toml", f"version=1\n[server]\nlisten=\"127.0.0.1:{self.http_port}\"\nworker_threads=1\ndrain_timeout_seconds=3\n[database]\nurl_file={json.dumps(str(runtime_url))}\nmax_connections=2\n[console]\nenabled=true\n")
        if getattr(self.args, "verify_runtime", False):
            key = self.root / "refresh.key"
            key.write_bytes(secrets.token_bytes(32))
            key.chmod(0o600)
            self.secrets.append(key)
            runtime.write_text(runtime.read_text() + f'''[identity]
registration_enabled=false
password_register_enabled=false
oauth_register_enabled=false
registration_disabled_methods=[]
email_verification_enabled=false
email_domain_restriction_enabled=false
email_domain_whitelist=[]
[identity.web]
origin="http://127.0.0.1:{self.http_port}"
refresh_key_file={json.dumps(str(key))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
''')
        self.unit = "lmm-offline-" + secrets.token_hex(6) + ".service"
        self.unit_file = Path(self.env["XDG_RUNTIME_DIR"]) / "systemd/user" / self.unit
        self.unit_file.parent.mkdir(parents=True, exist_ok=True)
        need(not self.unit_file.exists(), "unit_already_exists")
        self.unit_file.write_text(f"[Unit]\nDescription=Disposable LMM offline test\n[Service]\nExecStart={self.args.previous_core} --config {runtime} serve\nKillMode=control-group\nTimeoutStopSec=15\n")
        self.run("daemon-reload", ["systemctl", "--user", "daemon-reload"])
        self.run("old-core-start", ["systemctl", "--user", "start", self.unit])
        deadline = time.monotonic() + 20
        while True:
            try:
                connection = http.client.HTTPConnection("127.0.0.1", self.http_port, timeout=1)
                connection.request("GET", "/health/live")
                response = connection.getresponse()
                response.read()
                connection.close()
                if response.status == 200:
                    break
            except OSError:
                pass
            need(time.monotonic() < deadline, "old_core_not_live")
            time.sleep(.05)
        self.authority = self.cluster("authority")
        self.sql(self.authority, "CREATE ROLE history_owner NOLOGIN; CREATE ROLE history_writer LOGIN; GRANT CREATE ON DATABASE postgres TO history_owner")
        authority_id = hashlib.sha256(secrets.token_bytes(32)).hexdigest()
        history_install = self.config("history-install", self.url(self.authority, "postgres", "postgres"))
        self.run("history-install", [self.args.core, "--config", history_install, "upgrade-history"],
                 json.dumps({"action": "install", "authority_id": authority_id, "owner": "history_owner", "writer": "history_writer"}).encode())
        target_url = self.url(self.target, "lmm_core_installer", "coretarget", self.installer_password)
        history_url = self.url(self.authority, "history_writer", "postgres")
        if self.args.mode == "target-ack":
            sock = Path(tempfile.mkdtemp(prefix="lmm-off-proxy-", dir="/tmp"))
            self.proxy_directory = sock
            proxy = self.fault.Proxy(self.target["socket"] / f".s.PGSQL.{self.target['port']}", "lmm_core_installer", "coretarget",
                                     writer_sql=b"INSERT INTO core_meta.offline_upgrades", label="offline", listen_socket=sock / ".s.PGSQL.55602")
            self.proxies.append(proxy)
            target_url = f"postgresql://lmm_core_installer:{self.installer_password}@localhost/coretarget?host={sock}&port=55602&sslmode=disable"
            proxy.arm()
        elif self.args.mode == "history-ack":
            proxy = self.fault.Proxy(self.authority["socket"] / f".s.PGSQL.{self.authority['port']}", "history_writer", "postgres",
                                     writer_sql=b"lmm_upgrade_history.advance", label="history")
            self.proxies.append(proxy)
            history_url = f"postgresql://history_writer@127.0.0.1:{proxy.server_address[1]}/postgres?sslmode=disable"
        self.config_path = self.config("operator", target_url)
        management = self.file("management.url", self.url(self.target, "postgres", "postgres"))
        history = self.file("history.url", history_url)
        probe = self.file("probe.url", self.url(self.target, "lmm_core_runtime", "coretarget"))
        self.secrets += [management, history, probe]
        identity = json.loads(self.sql(self.target, "SELECT json_build_object('system',system_identifier::text,'oid',(SELECT oid::bigint FROM pg_database WHERE datname='coretarget')) FROM pg_control_system()"))
        self.request = {"version": 3, "action": "apply", "operation_id": "fixture-" + secrets.token_hex(16),
                        "candidate_sha256": sha(self.args.core), "authority_id": authority_id,
                        "history_url_file": str(history), "management_url_file": str(management), "runtime_probe_url_file": str(probe),
                        "hba_file": str(self.target["hba"]), "runtime": {"console": True, "assistant_agent": False},
                        "services": [{"name": self.unit, "scope": "user", "executable_sha256": sha(self.args.previous_core)}],
                        "bridge": {"version": 3, "procedure": "core-offline-upgrade-v3", "predecessor": "fresh_v6_core27",
                                   "predecessor_manifest_sha256": "59bc164463df00df11a253feecbf9e08dd914e6e3cc7fcb3af68d193287eb005",
                                   "predecessor_catalog_sha256": "acbee20015ea1397adfda06232f25c811956823e00b15e574d74794702f418e5",
                                   "successor_manifest_sha256": sha(self.args.source / "services/core/src/upgrade_offline/target.json"),
                                   "database_name": "coretarget", "database_oid": identity["oid"], "system_identifier": identity["system"],
                                   "installer": "lmm_core_installer", "owner": "lmm_core_owner", "runtime": "lmm_core_runtime", "extensions": "lmm_extensions"}}

    def runtime_checkpoint(self):
        """Fixture-only opening, not product finalization or permission to open a site."""
        need(self.args.mode == "none", "runtime_checkpoint_requires_confirmed_apply")
        gate = self.target["hba"].read_bytes()
        password = secrets.token_hex(24)
        self.runtime_password = password
        log = self.root / "upgraded-runtime.log"
        proc = None
        observations = []

        def request(method, path, body=None, token=None):
            headers = {"Origin": f"http://127.0.0.1:{self.http_port}"}
            if body is not None:
                headers["Content-Type"] = "application/json"
            if token:
                headers["Authorization"] = "Bearer " + token
            conn = http.client.HTTPConnection("127.0.0.1", self.http_port, timeout=5)
            try:
                conn.request(method, path, json.dumps(body) if body is not None else None, headers)
                response = conn.getresponse()
                payload = response.read()
                observations.append({"method": method, "path": path, "status": response.status})
                return response.status, json.loads(payload)
            finally:
                conn.close()

        try:
            self.sql(self.target, f"SET password_encryption='scram-sha-256'; ALTER ROLE lmm_core_runtime LOGIN PASSWORD '{password}'; GRANT CONNECT ON DATABASE coretarget TO lmm_core_runtime")
            self.file("runtime.url", self.url(self.target, "lmm_core_runtime", "coretarget", password))
            self.target["hba"].write_bytes(b'local "coretarget" "lmm_core_runtime" scram-sha-256\n' + gate)
            self.sql(self.target, "SELECT pg_reload_conf(); ALTER DATABASE coretarget ALLOW_CONNECTIONS true")
            with log.open("wb") as output:
                proc = subprocess.Popen([str(self.args.core), "--config", str(self.root / "runtime.toml"), "serve"],
                                        stdout=output, stderr=subprocess.STDOUT, env=self.env)
                deadline = time.monotonic() + 20
                while True:
                    need(proc.poll() is None, "upgraded_runtime_exited")
                    try:
                        status, _ = request("GET", "/health/live")
                        if status == 200:
                            break
                    except OSError:
                        pass
                    need(time.monotonic() < deadline, "upgraded_runtime_not_live")
                    time.sleep(.05)
                actual = sha(Path(f"/proc/{proc.pid}/exe"))
                need(actual == sha(self.args.core), "upgraded_runtime_artifact_mismatch")
                need(self.sql(self.target, "SELECT session_user=current_user AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolbypassrls) AND NOT pg_has_role(rolname,'lmm_core_owner','MEMBER') AND NOT has_table_privilege('core_meta.offline_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') AND NOT has_table_privilege('core_meta.console_revisions','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') FROM pg_roles WHERE rolname=current_user", "coretarget", "lmm_core_runtime") == "t", "runtime_authority_not_restricted")
                status, auth = request("POST", "/api/user/login", {"username": "offlineowner", "password": self.owner_password})
                need(status == 200 and auth.get("success") is True, "upgraded_password_login_failed")
                token = auth["data"]["access_token"]
                value = "Saved by the actual upgraded runtime"
                status, saved = request("PUT", "/api/option/", {"key": "Notice", "value": value}, token)
                need(status == 200 and saved.get("success") is True and saved.get("saved") is True, "upgraded_runtime_save_failed")
                status, current = request("GET", "/api/option/", token=token)
                need(status == 200 and current.get("success") is True, "upgraded_runtime_read_failed")
                need({row["key"]: row["value"] for row in current["data"]}.get("Notice") == value, "upgraded_runtime_value_not_persisted")
                proc.terminate()
                need(proc.wait(timeout=10) == 0, "upgraded_runtime_stop_failed")
                self.sql(self.target, "REVOKE SELECT ON core_meta.schema_contract FROM lmm_core_runtime", "coretarget", "lmm_core_installer", True)
                try:
                    denied = self.run("missing-public-contract-denied", [self.args.core, "--config", self.root / "runtime.toml", "serve"], expected=1)
                    need(b"MissingPrivileges" in denied.stderr, "missing_public_contract_not_rejected")
                finally:
                    self.sql(self.target, "GRANT SELECT ON core_meta.schema_contract TO lmm_core_runtime", "coretarget", "lmm_core_installer", True)
                for privilege in ["SELECT", "SELECT(operation_id)"]:
                    self.sql(self.target, f"GRANT {privilege} ON core_meta.offline_upgrades TO lmm_core_runtime", "coretarget", "lmm_core_installer", True)
                    try:
                        denied = self.run("private-receipt-overgrant-denied", [self.args.core, "--config", self.root / "runtime.toml", "serve"], expected=1)
                        need(b"UnsafePrivileges" in denied.stderr, "private_receipt_overgrant_not_rejected")
                    finally:
                        self.sql(self.target, f"REVOKE {privilege} ON core_meta.offline_upgrades FROM lmm_core_runtime", "coretarget", "lmm_core_installer", True)
                self.save("runtime-checkpoint.json", {"status": "PASS", "pid": proc.pid, "actual_executable_sha256": actual,
                          "http": observations, "database_role": "lmm_core_runtime", "restricted_role": True,
                          "private_upgrade_receipt_access": False, "missing_public_contract_rejected": True,
                          "private_receipt_table_and_column_overgrants_rejected": True,
                          "scope": "fixture administrative opening and actual upgraded Core HTTP; no product finalization or production permission"})
        finally:
            if proc is not None and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
            self.target["hba"].write_bytes(gate)
            self.sql(self.target, "SELECT pg_reload_conf(); ALTER ROLE lmm_core_runtime NOLOGIN; REVOKE CONNECT ON DATABASE coretarget FROM lmm_core_runtime; ALTER DATABASE coretarget ALLOW_CONNECTIONS false")

    def exercise(self):
        self.setup()
        if self.args.mode == "history-ack":
            output = self.root / "history-ack-cli.log"
            with output.open("wb") as file:
                proc = subprocess.Popen([str(self.args.core), "--config", str(self.config_path), "upgrade-offline"],
                                        stdin=subprocess.PIPE, stdout=file, stderr=subprocess.STDOUT, env=self.env)
                proc.stdin.write(json.dumps(self.request).encode())
                proc.stdin.close()
                deadline = time.monotonic() + 40
                while self.sql(self.authority, "SELECT coalesce(max(phase),'none') FROM lmm_upgrade_history.operations") != "started":
                    need(proc.poll() is None, "cli_exited_before_started")
                    need(time.monotonic() < deadline, "started_not_observed")
                    time.sleep(.01)
                self.proxies[0].arm()
                need(proc.wait(timeout=120) != 0, "history_ack_not_unknown")
        else:
            _, result = self.cli("apply", self.config_path, self.request, 0 if self.args.mode == "none" else 1)
            need(result["status"] == ("committed" if self.args.mode == "none" else "unconfirmed"), "apply_status")
        if self.proxies:
            need(self.proxies[0].dropped.is_set(), "fault_not_really_injected")
            need(not self.proxies[0].errors, "fault_proxy_error")
            self.save("fault.json", {"events": self.proxies[0].events, "errors": self.proxies[0].errors})
        self.assert_closed()
        need(self.target_sql(self.facts_sql) == self.before, "old_rows_changed")
        need(self.target_sql("SELECT count(*) FROM core_meta.offline_upgrades") == "1", "local_receipt_missing_or_duplicated")
        recover = copy.deepcopy(self.request)
        recover["action"] = "recover"
        _, observed = self.cli("recover", self.config_path, recover)
        need(observed["status"] == ("committed_receipt_recovered" if self.args.mode == "target-ack" else "committed_history_observed"), "recovery_classification")
        need(observed["target_contract_matches"] is True, "recovery_contract_mismatch")
        if getattr(self.args, "verify_runtime", False):
            self.runtime_checkpoint()
            self.assert_closed()
        self.cli("repeat-apply", self.config_path, self.request, 1)
        other = copy.deepcopy(self.request)
        other["operation_id"] = "different-" + secrets.token_hex(16)
        self.cli("different-operation", self.config_path, other, 1)
        self.stop(self.authority)
        self.start(self.authority)
        need(self.sql(self.authority, "SELECT phase FROM lmm_upgrade_history.operations") == "committed", "authority_restart_lost_history")
        hba = sha(self.target["hba"])
        self.stop(self.target)
        self.target["data"].rename(self.target["directory"] / "committed-data")
        shutil.copytree(self.backup, self.target["data"])
        self.start(self.target)
        need(sha(self.target["hba"]) == hba, "physical_restore_replaced_external_hba")
        _, restored = self.cli("recover-physical-restore", self.config_path, recover)
        need(restored["status"] == "database_diverged_from_committed_history", "old_backup_misreported_rollback")
        need(restored["observed_receipt"] is False and restored["retry_allowed"] is False and restored["site_open_allowed"] is False, "restore_allowed_replay_or_open")
        self.cli("apply-after-restore", self.config_path, self.request, 1)
        self.assert_closed()
        need(self.target_sql("SELECT to_regclass('core_meta.offline_upgrades') IS NULL") == "t", "restore_replayed_ddl")
        self.save("result.json", {"status": "PASS", "mode": self.args.mode, "core_sha256": sha(self.args.core),
                                  "previous_core_sha256": sha(self.args.previous_core), "scope": "actual CLI, synthetic v6/27 predecessor, target DB and named Core unit; no site opening or production proof"})

    def close(self):
        for proxy in self.proxies:
            proxy.close()
        if self.unit:
            self.run("unit-cleanup-stop", ["systemctl", "--user", "stop", self.unit], expected=None)
            self.run("unit-cleanup-unmask", ["systemctl", "--user", "unmask", self.unit], expected=None)
        if self.unit_file:
            self.unit_file.unlink(missing_ok=True)
            self.run("cleanup-daemon-reload", ["systemctl", "--user", "daemon-reload"])
        for cluster in reversed(self.clusters):
            self.stop(cluster)
        result=self.root / "result.json"
        if result.exists() and json.loads(result.read_text()).get("status")=="PASS":
            for path in self.secrets:
                path.unlink(missing_ok=True)


def main():
    os.umask(0o077)
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--core", type=Path, required=True)
    p.add_argument("--previous-core", type=Path, required=True)
    p.add_argument("--source", type=Path, required=True)
    p.add_argument("--reference", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--mode", choices=["none", "target-ack", "history-ack"], default="none")
    p.add_argument("--verify-runtime", action="store_true", help="After confirmed apply, explicitly open only the disposable fixture and exercise the actual new Core; not site finalization")
    args = p.parse_args()
    for name in ["core", "previous_core", "source", "reference"]:
        setattr(args, name, getattr(args, name).resolve(strict=True))
    with open("/tmp/lmm-delegate/heavy-build.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        fixture = Fixture(args)
        try:
            fixture.exercise()
        except BaseException as error:
            fixture.save("result.json", {"status": "FAIL", "error": str(error), "mode": args.mode})
            raise
        finally:
            fixture.close()


if __name__ == "__main__":
    main()
