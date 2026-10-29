#!/usr/bin/env python3
"""Independent Console acceptance: existing non-test Core, real CLI/HTTP/PG.

No builds and no product imports. See README.md for freeze and resource gates.
Only a new private local cluster is permitted; no external database argument.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time
from urllib.parse import urlencode

from pg_fault import Proxy


class Failure(Exception):
    pass


def need(condition, label):
    if not condition:
        raise Failure(label)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Process:
    def __init__(self, runner, label, argv):
        self.label, self.output, self.overflow = label, bytearray(), False
        self.proc = subprocess.Popen(list(map(str, argv)), cwd=runner.source, env=runner.env,
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, start_new_session=True)
        self.record = {"label": label, "argv": list(map(str, argv)), "pid": self.proc.pid}
        runner.commands.append(self.record)
        runner.processes.append(self)
        self.reader = threading.Thread(target=self.drain, daemon=True)
        self.reader.start()

    def drain(self):
        while block := self.proc.stdout.read(8192):
            if len(self.output) + len(block) <= 4 * 1024 * 1024:
                self.output.extend(block)
            else:
                self.overflow = True

    def stop(self, sig=signal.SIGTERM):
        if self.proc.poll() is None:
            os.killpg(self.proc.pid, sig)
        try:
            self.proc.wait(15)
        except subprocess.TimeoutExpired:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait(5)
            self.record["forced_kill"] = True
        self.reader.join(5)
        self.record["exit_code"] = self.proc.returncode
        need(not self.reader.is_alive() and not self.overflow, "process_log_capture_incomplete")


CASES = ["freeze_and_resources", "fresh_cli_and_runtime", "login_and_permissions",
         "save_aliases_validation", "concurrent_cas", "audit_failure_rollback",
         "commit_response_loss", "restart_persistence", "sql_trust_boundary",
         "secrets_and_final_binding"]


class Runner:
    def __init__(self, args):
        self.args = args
        self.source, self.core = args.source.resolve(strict=True), args.core.resolve(strict=True)
        self.evidence = args.evidence.absolute()
        need(not self.evidence.resolve().is_relative_to(self.source), "evidence_outside_frozen_source")
        self.evidence.mkdir(mode=0o700, exist_ok=False)
        self.env = {"PATH": "/usr/bin:/bin", "HOME": str(Path.home()), "LANG": "C", "TZ": "UTC"}
        self.results = {case: "NOT_RUN" for case in CASES}
        self.events, self.commands, self.processes, self.sensitive = [], [], [], set()
        self.work = self.sockdir = self.pg = self.main = self.proxy = None
        self.raw_logs = []
        self.lock = None
        self.case = None

    def save(self, name, data):
        (self.evidence / name).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")

    def secret(self, value):
        if isinstance(value, str) and value:
            self.sensitive.add(value)
        return value

    def file(self, name, value):
        path = self.work / name
        path.write_bytes(value if isinstance(value, bytes) else value.encode())
        path.chmod(0o600)
        return path

    def command(self, label, argv, *, env=None, data=None, expected=0, timeout=60):
        started = time.monotonic()
        result = subprocess.run(list(map(str, argv)), cwd=self.source, env=env or self.env,
                                input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=timeout)
        self.commands.append({"label": label, "argv": list(map(str, argv)),
                              "exit_code": result.returncode,
                              "seconds": round(time.monotonic() - started, 3)})
        self.raw_logs.append((label, result.stdout + result.stderr))
        need(result.returncode == expected, label + "_exit")
        return result.stdout

    def sql(self, label, statement, role="fixture_admin", expected=0):
        env = {**self.env, "PGHOST": str(self.sockdir), "PGPORT": "5432",
               "PGUSER": role, "PGDATABASE": "console_acceptance", "PGSSLMODE": "disable"}
        return self.command(label, ["psql", "-X", "-A", "-t", "-q", "-v", "ON_ERROR_STOP=1"],
                            env=env, data=statement.encode(), expected=expected).decode().strip()

    def snapshot(self):
        # Snapshot comparisons include revision, timestamp, actor and credential.
        # Return only in memory; evidence contains a digest, never raw row values.
        result = json.loads(self.sql("snapshot", """
SELECT json_build_object(
 'options',(SELECT coalesce(json_agg(x),'[]') FROM
   (SELECT key,value,revision,updated_at::text FROM core_console.options ORDER BY key) x),
 'audit',(SELECT coalesce(json_agg(x),'[]') FROM
   (SELECT id,actor_user_id,credential_id,target FROM core_identity.audit
     WHERE action='console.options.update' ORDER BY id) x));
"""))
        self.events.append({"snapshot_sha256": hashlib.sha256(json.dumps(result, sort_keys=True).encode()).hexdigest(),
                            "options": len(result["options"]), "audit": len(result["audit"])})
        return result

    def http(self, label, method, path, body=None, token=None, raw=None):
        headers = {"Origin": f"http://127.0.0.1:{self.port}"}
        if token:
            headers["Authorization"] = "Bearer " + token
        encoded = raw if raw is not None else (None if body is None else json.dumps(body).encode())
        if encoded is not None:
            headers["Content-Type"] = "application/json"
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=12)
        try:
            connection.request(method, path, body=encoded, headers=headers)
            response = connection.getresponse()
            payload = response.read(1024 * 1024 + 1)
            need(len(payload) <= 1024 * 1024, "http_size_limit")
            value = json.loads(payload) if payload else None
            received = {k.lower(): v for k, v in response.getheaders()}
            self.secret(received.get("set-cookie"))
            if isinstance(value, dict):
                self.secret(value.get("secret"))
                data = value.get("data")
                if isinstance(data, dict):
                    self.secret(data.get("access_token"))
                    if isinstance(data.get("session"), dict):
                        self.secret(data["session"].get("sid"))
            if path.startswith("/api/option"):
                need(received.get("cache-control") == "no-store", label + "_no_store")
                for secret in self.sensitive:
                    need(secret.encode() not in payload, label + "_secret_reflection")
                need(not any(x in payload for x in (b"postgresql://", b"SQLSTATE", b"core_identity.", b"core_console.")),
                     label + "_storage_details_leaked")
            self.events.append({"http": label, "method": method, "path": path, "status": response.status,
                                "code": value.get("code") if isinstance(value, dict) else None,
                                "no_store": received.get("cache-control") == "no-store"})
            return response.status, value
        finally:
            connection.close()

    def option(self, label, method, path, body=None, *, token=None, status=200, code=None, raw=None):
        actual, value = self.http(label, method, path, body, token, raw)
        need(actual == status, label + "_status")
        need(isinstance(value, dict) and value.get("success") is (status == 200), label + "_success_body")
        if code:
            need(value.get("code") == code and isinstance(value.get("message"), str), label + "_error_body")
        return value

    def binding(self):
        need(sha(self.core) == self.args.core_sha256, "frozen_binary_hash_drift")
        pins = json.loads(self.args.pins.read_text())
        need(isinstance(pins, dict) and len(pins) > 0, "empty_source_pins")
        for name, expected in pins.items():
            path = (self.source / name).resolve(strict=True)
            need(path.is_relative_to(self.source), "pin_outside_source")
            need(sha(path) == expected, "frozen_source_pin_drift")
        need("services/core/src/main.rs" in pins and "deployment/postgres/grant-core-runtime.sql" in pins,
             "manifest_missing_formal_main_or_grants")
        return {"core_sha256": sha(self.core), "pins_sha256": sha(self.args.pins), "input_count": len(pins),
                "freeze_sha256": sha(self.args.freeze), "source": str(self.source), "core": str(self.core),
                "test_inputs": {path.name: sha(path) for path in Path(__file__).parent.glob("*.py")},
                "provenance": "integrator supplied actual non-test Cargo dev main; not release or S11"}

    def freeze_and_resources(self):
        need(os.geteuid() != 0, "nonroot_runner_required")
        self.before = self.binding()
        self.save("binding-before.json", self.before)
        self.lock = open("/tmp/lmm-delegate/heavy-build.lock", "a")
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        memory = int(next(line.split()[1] for line in Path("/proc/meminfo").read_text().splitlines()
                          if line.startswith("MemAvailable:")))
        cgroup = Path("/sys/fs/cgroup") / Path("/proc/self/cgroup").read_text().strip().split("::", 1)[1].lstrip("/")
        limits = {name: (cgroup / name).read_text().strip() for name in
                  ("memory.max", "memory.high", "memory.swap.max", "cpu.max")}
        self.cgroup = cgroup
        self.save("resource-admission.json", {"available_kib": memory, "minimum_kib": 2097152,
                  "cgroup": str(cgroup), "limits": limits,
                  "memory_events": (cgroup / "memory.events").read_text()})
        need(memory >= 2097152, "2GiB_admission_closed")
        for name, limit in (("memory.max", 2**31), ("memory.high", 1536 * 1024**2), ("memory.swap.max", 4 * 1024**3)):
            need(limits[name] != "max" and int(limits[name]) <= limit, "cgroup_limit_" + name)
        quota, period = limits["cpu.max"].split()
        need(quota != "max" and int(quota) <= int(period), "one_cpu_limit_required")
        for tool in ("initdb", "postgres", "psql", "pg_isready"):
            need(shutil.which(tool, path=self.env["PATH"]), "missing_" + tool)
        self.command("space_df", ["df", "-h", self.evidence, "/tmp"])
        self.command("core_version", [self.core, "--version"])
        self.command("compiled_schema", [self.core, "schema-manifest"])

    def grants(self):
        self.command("official_grants", ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1",
                    "-v", "runtime_role=console_runtime", "-v", "enable_console=true",
                    "-v", "enable_web_identity=true", "-f", self.source / "deployment/postgres/grant-core-runtime.sql"],
                    env={**self.env, "PGHOST": str(self.sockdir), "PGPORT": "5432",
                         "PGUSER": "console_installer", "PGDATABASE": "console_acceptance",
                         "PGOPTIONS": "-c role=lmm_core_owner", "PGSSLMODE": "disable"})

    def start_core(self, label):
        self.main = Process(self, label, [self.core, "--config", self.config, "serve"])
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline:
            need(self.main.proc.poll() is None, "formal_main_exited_before_ready")
            try:
                status, body = self.http("ready", "GET", "/health/ready")
                if status == 200:
                    need(body.get("service") == "lmm-core" and body.get("status") == "ready", "formal_readiness")
                    need(all(body.get("capabilities", {}).get(k) is True for k in
                             ("identity", "legacy_web_auth", "console")), "formal_capabilities")
                    need(sha(Path(f"/proc/{self.main.proc.pid}/exe")) == self.args.core_sha256, "running_exe_hash")
                    self.events.append({"formal_main_pid": self.main.proc.pid, "exe_sha256": self.args.core_sha256})
                    return
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(.1)
        raise Failure("formal_readiness_timeout")

    def fresh_cli_and_runtime(self):
        self.work = Path(tempfile.mkdtemp(prefix=".lmm-console-process-", dir=Path.home()))
        self.sockdir = Path(tempfile.mkdtemp(prefix="lmm-cp-", dir="/tmp"))
        need(shutil.disk_usage(self.work).free >= 512 * 1024**2, "home_space_low")
        self.save("fixture-location.json", {"work": str(self.work), "socket": str(self.sockdir)})
        version = self.command("postgres_version", ["postgres", "--version"])
        need(re.search(rb"\b18\.", version), "postgres18_required")
        self.command("initdb", ["initdb", "-D", self.work / "data", "-U", "fixture_admin",
                     "--no-locale", "--encoding=UTF8", "--auth-local=trust", "--auth-host=reject", "--wal-segsize=1"])
        self.pg = Process(self, "postgres", ["postgres", "-D", self.work / "data", "-c", "listen_addresses=",
                     "-c", f"unix_socket_directories={self.sockdir}", "-c", "unix_socket_permissions=0700",
                     "-c", "shared_buffers=16MB", "-c", "max_connections=24", "-c", "min_wal_size=2MB",
                     "-c", "max_wal_size=128MB", "-c", "log_statement=none", "-c", "log_min_error_statement=panic",
                     "-c", "log_parameter_max_length_on_error=0"])
        for _ in range(100):
            if (self.sockdir / ".s.PGSQL.5432").exists():
                probe = subprocess.run(["pg_isready", "-q", "-h", str(self.sockdir), "-U", "fixture_admin"], env=self.env)
                if probe.returncode == 0:
                    break
            need(self.pg.proc.poll() is None, "postgres_start_failed")
            time.sleep(.1)
        else:
            raise Failure("postgres_readiness_timeout")
        self.command("roles_and_database", ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-h", self.sockdir,
                     "-U", "fixture_admin", "-d", "postgres"], data=b"""
CREATE ROLE lmm_core_owner NOLOGIN;
CREATE ROLE console_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE console_runtime LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT lmm_core_owner TO console_installer WITH INHERIT FALSE, SET TRUE;
CREATE DATABASE console_acceptance OWNER lmm_core_owner;
""")
        self.sql("database_boundary", "REVOKE CREATE,TEMPORARY ON DATABASE console_acceptance FROM PUBLIC; REVOKE CREATE ON SCHEMA public FROM PUBLIC;")
        self.proxy = Proxy(self.sockdir / ".s.PGSQL.5432", "console_runtime", "console_acceptance")
        owner_url = "postgresql://console_installer@localhost:5432/console_acceptance?" + urlencode(
            {"host": str(self.sockdir), "sslmode": "disable", "options": "-c role=lmm_core_owner"})
        owner_file = self.file("owner.url", owner_url)
        owner_config = self.file("owner.toml", "version=1\n[database]\nurl_file=" + json.dumps(str(owner_file)) + "\n")
        self.command("formal_init_db", [self.core, "--config", owner_config, "init-db"])
        self.password = self.secret("P!" + secrets.token_hex(16))
        receipt = json.loads(self.command("formal_first_owner", [self.core, "--config", owner_config, "create-first-owner"],
                     data=json.dumps({"username": "consoleowner", "display_name": "Local Console owner", "password": self.password}).encode()))
        need(receipt.get("setup_completed") is True, "first_owner_receipt")
        self.owner_id = receipt["user_id"]
        self.grants()
        need(self.sql("schema_v6", "SELECT version FROM core_meta.schema_contract WHERE singleton") == "6", "identity_not_v6")
        need(self.sql("component27", "SELECT count(*) FROM core_meta.component_contract") == "27", "not_27_components")
        identity = json.loads(self.sql("pg_identity", "SELECT json_build_object('system_id',system_identifier::text,'data_directory',current_setting('data_directory'),'database',current_database(),'listen',current_setting('listen_addresses')) FROM pg_control_system()"))
        need(Path(identity["data_directory"]).resolve() == (self.work / "data").resolve() and identity["listen"] == "", "pg_identity_mismatch")
        self.save("postgres-identity.json", identity)
        need(self.sql("runtime_boundary", """SELECT NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)
 AND NOT pg_has_role(rolname,'lmm_core_owner','MEMBER') FROM pg_roles WHERE rolname='console_runtime'""") == "t", "runtime_privileged")
        need(self.sql("runtime_identity", "SELECT session_user=current_user AND current_user='console_runtime'", "console_runtime") == "t", "runtime_set_role")
        need(self.sql("installation_object_owners", """SELECT bool_and(pg_get_userbyid(c.relowner)='lmm_core_owner')
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core_%'""") == "t",
             "core_objects_not_owned_by_installer_owner")
        runtime = self.file("runtime.url", f"postgresql://console_runtime@127.0.0.1:{self.proxy.server_address[1]}/console_acceptance?sslmode=disable")
        key = self.file("refresh.key", secrets.token_bytes(32))
        self.port = free_port()
        self.config = self.file("core.toml", f'''version=1
[server]
listen="127.0.0.1:{self.port}"
worker_threads=1
drain_timeout_seconds=10
[database]
url_file={json.dumps(str(runtime))}
max_connections=4
[console]
enabled=true
[identity]
registration_enabled=true
password_register_enabled=true
oauth_register_enabled=false
registration_disabled_methods=[]
email_verification_enabled=false
email_domain_restriction_enabled=false
email_domain_whitelist=[]
[identity.web]
origin="http://127.0.0.1:{self.port}"
refresh_key_file={json.dumps(str(key))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
''')
        self.hidden = self.secret("private-option-" + secrets.token_hex(16))
        self.sql("hidden_option_fixture", "INSERT INTO core_console.options(key,value) VALUES ('OAuthSecret','" + self.hidden + "')")
        self.start_core("formal_main")
        need(int(self.sql("actual_runtime_connections", "SELECT count(*) FROM pg_stat_activity WHERE datname='console_acceptance' AND usename='console_runtime'")) > 0,
             "formal_main_runtime_connection_missing")

    def login_and_permissions(self):
        status, body = self.http("administrator_password_login", "POST", "/api/user/login",
                                 {"username": "consoleowner", "password": self.password})
        need(status == 200 and body.get("success") is True, "admin_login_failed")
        self.token = self.secret(body["data"]["access_token"])
        status, principal = self.http("administrator_identity", "GET", "/core/v1/identity", token=self.token)
        need(status == 200 and principal["platform_level"] == 6 and principal["user_id"] == self.owner_id,
             "admin_identity_not_L6")
        self.credential_id = principal["credential_id"]
        self.user_password = self.secret("Us!" + secrets.token_hex(8))
        status, user = self.http("ordinary_native_register", "POST", "/core/v1/auth/register",
                                {"login_name": "consoleordinary", "password": self.user_password})
        need(status == 403 and user.get("error", {}).get("code") == "identity_forbidden",
             "native_registration_must_remain_disabled_with_web")
        status, user = self.http("ordinary_web_register", "POST", "/api/user/register",
                                {"username": "consoleordinary", "password": self.user_password, "accepted_legal": True})
        need(status == 200 and user.get("success") is True, "ordinary_web_register_failed")
        status, user = self.http("ordinary_web_login", "POST", "/api/user/login",
                                {"username": "consoleordinary", "password": self.user_password})
        need(status == 200 and user.get("success") is True, "ordinary_web_login_failed")
        user_token = self.secret(user["data"]["access_token"])
        status, key = self.http("administrator_key", "POST", "/core/v1/keys",
                               {"owner": principal["owner"], "ttl_seconds": 3600}, self.token)
        need(status == 201, "key_issue_failed")
        key_token = self.secret(key["secret"])
        before = self.snapshot()
        routes = [("GET", "/api/option", None), ("PUT", "/api/option", {"key": "Notice", "value": "denied"}),
                  ("POST", "/api/option/bulk", {"values": {"Notice": "denied"}}),
                  ("POST", "/api/option/validate", {"values": {"Notice": "denied"}})]
        for label, token, status, code in [("anonymous", None, 401, "AUTH_UNAUTHORIZED"),
                ("ordinary", user_token, 403, "SYSTEM_OPTIONS_FORBIDDEN"),
                ("key", key_token, 403, "SYSTEM_OPTIONS_FORBIDDEN")]:
            for method, path, body in routes:
                self.option(label + "_" + path, method, path, body, token=token, status=status, code=code)
        status, session = self.http("revocation_session_login", "POST", "/core/v1/auth/login",
                                   {"login_name": "consoleowner", "password": self.password})
        need(status == 200, "second_login_failed")
        revoked = self.secret(session["secret"])
        need(self.http("session_revoke", "DELETE", f"/core/v1/credentials/{session['id']}", token=self.token)[0] == 204,
             "revoke_failed")
        for method, path, body in routes:
            self.option("revoked_" + path, method, path, body, token=revoked, status=401, code="AUTH_UNAUTHORIZED")
        need(self.snapshot() == before, "authorization_denials_mutated_options_or_audit")

    def change(self, before, after, expected):
        rows0 = {r["key"]: r for r in before["options"]}
        rows1 = {r["key"]: r for r in after["options"]}
        need(set(rows1) == set(rows0) | set(expected), "unexpected_option_keys")
        need(len(after["audit"]) == len(before["audit"]) + 1, "exactly_one_http_audit_required")
        need(after["audit"][:-1] == before["audit"], "old_audits_changed")
        audit = after["audit"][-1]
        need(audit["actor_user_id"] == self.owner_id and audit["credential_id"] == self.credential_id, "audit_actor_binding")
        changes = audit["target"]["changes"]
        need(len(changes) == len(expected) and {c["key"] for c in changes} == set(expected), "audit_changed_keys")
        for key, row in rows1.items():
            old = rows0.get(key)
            if key not in expected:
                need(row == old, "unrelated_option_mutated")
                continue
            revision = old["revision"] + 1 if old else 1
            need(row["value"] == expected[key] and row["revision"] == revision, "value_revision_mismatch")
            change = next(c for c in changes if c["key"] == key)
            need(change["before"] == ([old["value"], old["revision"]] if old else None), "audit_before")
            need(change["after"] == [expected[key], revision], "audit_after")

    def save_aliases_validation(self):
        before = self.snapshot()
        body = self.option("save_notice", "PUT", "/api/option/", {"key": "Notice", "value": "hello"}, token=self.token)
        need(body.get("saved") is True, "notice_saved_flag")
        saved = self.snapshot()
        self.change(before, saved, {"Notice": "hello"})
        self.option("no_op", "PUT", "/api/option", {"key": "Notice", "value": "hello"}, token=self.token)
        need(self.snapshot() == saved, "noop_changed_revision_timestamp_or_audit")
        self.announcement = json.dumps([{"id": 1, "content": "independent-local", "publishDate": "2026-10-29T00:00:00Z"}], separators=(",", ":"))
        self.option("save_announcement_alias", "PUT", "/api/option", {"key": "Announcements", "value": self.announcement}, token=self.token)
        aliases = self.snapshot()
        self.change(saved, aliases, {"Announcements": self.announcement, "console_setting.announcements": self.announcement})
        for path in ("/api/option", "/api/option/"):
            result = self.option("read_after_save", "GET", path, token=self.token)
            data = {item["key"]: item["value"] for item in result["data"]}
            need(data.get("Notice") == "hello" and data.get("Announcements") == self.announcement
                 and data.get("console_setting.announcements") == self.announcement, "read_after_save_values")
            need("OAuthSecret" not in data and result["capabilities"]["read_only"] is False, "public_projection")
        validated = self.option("validate_only", "POST", "/api/option/validate", {"values": {"Notice": "not-saved"}}, token=self.token)
        need(validated.get("saved") is False, "validate_saved_flag")
        invalid = [({"values": {"Notice": "partial", "OAuthSecret": self.hidden}}, 409, "SYSTEM_OPTION_NOT_WRITABLE"),
                   ({"values": {"Announcements": "not-json"}}, 400, "SYSTEM_OPTIONS_INVALID"),
                   ({"values": {"Announcements": "[]", "console_setting.announcements": self.announcement}}, 400, "SYSTEM_OPTIONS_INVALID"),
                   ({"values": {"Notice": "stale"}, "expected_values": {"Notice": "wrong"}}, 409, "SYSTEM_OPTIONS_CONFLICT"),
                   ({"values": {"Notice": "wrong-target"}, "expected_values": {"Announcements": "[]"}}, 400, "SYSTEM_OPTIONS_INVALID")]
        for index, (body, status, code) in enumerate(invalid):
            self.option(f"invalid_{index}", "POST", "/api/option/bulk", body, token=self.token, status=status, code=code)
        self.option("malformed_body", "PUT", "/api/option", token=self.token, raw=b'{"key":', status=400, code="SYSTEM_OPTIONS_INVALID")
        self.option("non_string", "PUT", "/api/option", {"key": "Notice", "value": 3}, token=self.token, status=400, code="SYSTEM_OPTIONS_INVALID")
        need(self.snapshot() == aliases, "invalid_or_validate_mutated_state")

    def concurrent_cas(self):
        for field, old in (("Notice", "hello"), ("Announcements", self.announcement)):
            before = self.snapshot()
            barrier = threading.Barrier(2)
            values = ["race-a", "race-b"] if field == "Notice" else [
                json.dumps([{"id": 2 + i, "content": f"race-{i}", "publishDate": "2026-10-29T00:00:00Z"}]) for i in range(2)]

            def race(index):
                barrier.wait(timeout=5)
                key = "console_setting.announcements" if field == "Announcements" and index == 1 else field
                return self.http("concurrent_compare_and_save", "POST", "/api/option/bulk",
                                 {"values": {key: values[index]}, "expected_values": {key: old}}, self.token)

            with ThreadPoolExecutor(max_workers=2) as pool:
                results = list(pool.map(race, (0, 1)))
            need(sorted(status for status, _ in results) == [200, 409], "CAS_single_winner")
            winner = next(i for i, (status, _) in enumerate(results) if status == 200)
            need(results[winner][1].get("saved") is True, "CAS_winner_body")
            need(results[1 - winner][1].get("success") is False and results[1 - winner][1].get("code") == "SYSTEM_OPTIONS_CONFLICT", "CAS_loser_body")
            expected = {field: values[winner]} if field == "Notice" else {"Announcements": values[winner], "console_setting.announcements": values[winner]}
            self.change(before, self.snapshot(), expected)

    def audit_failure_rollback(self):
        before = self.snapshot()
        self.sql("fixture_revoke_audit", "SET ROLE lmm_core_owner; REVOKE INSERT ON core_identity.audit FROM console_runtime")
        try:
            self.option("audit_failure", "POST", "/api/option/bulk", {"values": {"Notice": "must-rollback", "Announcements": "[]"}},
                        token=self.token, status=503, code="SYSTEM_OPTIONS_STORAGE_UNAVAILABLE")
            need(self.snapshot() == before, "audit_failure_did_not_rollback_all_options")
        finally:
            self.grants()

    def commit_response_loss(self):
        before = self.snapshot()
        self.proxy.arm()
        body = self.option("lost_commit_response", "PUT", "/api/option", {"key": "Notice", "value": "committed-no-reply"},
                           token=self.token, status=503, code="SYSTEM_OPTIONS_COMMIT_UNKNOWN")
        need(self.proxy.dropped.wait(3) and not self.proxy.errors, "commit_response_injection_not_proven")
        need(not any(text in body["message"].lower() for text in ("no changes were committed", "rollback", "rolled back")), "unknown_claimed_rollback")
        need(body.get("saved") is not True, "unknown_claimed_success")
        self.change(before, self.snapshot(), {"Notice": "committed-no-reply"})
        read = self.option("resolve_by_fresh_read", "GET", "/api/option", token=self.token)
        need(any(row["key"] == "Notice" and row["value"] == "committed-no-reply" for row in read["data"]), "post_unknown_read")
        self.save("commit-fault.json", {"events": self.proxy.events, "errors": self.proxy.errors,
                  "observed_database_outcome": "COMMITTED", "observed_http_outcome": "503 SYSTEM_OPTIONS_COMMIT_UNKNOWN",
                  "automatic_retry": False})

    def restart_persistence(self):
        before = self.snapshot()
        self.main.stop()
        need(self.main.proc.returncode == 0, "formal_main_shutdown_failed")
        self.start_core("formal_main_restart")
        self.option("read_after_process_restart", "GET", "/api/option", token=self.token)
        need(self.snapshot() == before, "restart_changed_data_or_audit")

    def sql_trust_boundary(self):
        self.main.stop()
        before = self.snapshot()
        denied = ["INSERT INTO core_console.options(key,value) VALUES ('Notice','denied')",
                  "UPDATE core_console.options SET value='denied' WHERE key='Notice'",
                  "DELETE FROM core_console.options", "TRUNCATE core_console.options",
                  "CREATE TABLE core_console.denied(x int)", "SET ROLE lmm_core_owner",
                  "SELECT * FROM core_meta.console_revisions"]
        for index, statement in enumerate(denied):
            self.sql(f"runtime_sql_denied_{index}", "\\set VERBOSITY sqlstate\n" + statement, "console_runtime", expected=3)
            need(b"42501" in self.raw_logs[-1][1], "SQL_denial_wrong_sqlstate")
        self.sql("runtime_unsupported_function", "\\set VERBOSITY sqlstate\nSELECT core_console.write_public_option('OAuthSecret','denied')", "console_runtime", expected=3)
        need(b"22023" in self.raw_logs[-1][1], "unsupported_key_wrong_sqlstate")
        need(self.snapshot() == before, "denied_SQL_mutated_state")
        # Explicitly observe the documented trusted-application exception.
        # This is NOT database-enforced HTTP validation/audit acceptance.
        self.sql("trusted_SQL_function", "SELECT core_console.write_public_option('Announcements','not-json-direct-sql')", "console_runtime")
        after = self.snapshot()
        rows = {row["key"]: row for row in after["options"]}
        need(rows["Announcements"]["value"] == "not-json-direct-sql" and after["audit"] == before["audit"],
             "trusted_SQL_observation_changed_review_contract")
        self.save("sql-trust-boundary.json", {"runtime_is_trusted_core_application": True,
                  "direct_table_DML_DDL_and_owner_role": "DENIED 42501",
                  "unsupported_function_key": "DENIED 22023", "allowed_function_without_http_audit": "OBSERVED",
                  "direct_sql_can_bypass_announcement_JSON_and_alias_consistency": True,
                  "database_enforced_end_user_audit": "NOT_PROVIDED; outside this trusted-role contract",
                  "HTTP_audit": "successful changed writes atomic with actor/credential and before/after; no-op/denials unchanged"})

    def secrets_and_final_binding(self):
        need(not self.proxy.errors, "proxy_errors")
        for process in self.processes:
            if process.label.startswith("formal_main"):
                process.stop()
                for secret in self.sensitive:
                    need(secret.encode() not in process.output, "secret_in_Core_process_log")
        after = self.binding()
        need(after == self.before, "input_binding_changed")
        self.save("binding-after.json", after)

    def finish(self):
        cleanup_errors = []
        for process in reversed(self.processes):
            try:
                process.stop(signal.SIGINT if process is self.pg else signal.SIGTERM)
            except Exception:
                cleanup_errors.append("owned_process_cleanup_failed")
        if self.proxy:
            self.proxy.close()
            self.save("proxy-events.json", {"events": self.proxy.events, "errors": self.proxy.errors})
        for label, raw in self.raw_logs + [(p.label, bytes(p.output)) for p in self.processes]:
            # Persist redacted transcripts only. SQL rows/CLI credentials stay private.
            if label == "snapshot" or label == "hidden_option_fixture":
                continue
            text = raw.decode(errors="replace")
            for secret in sorted(self.sensitive, key=len, reverse=True):
                text = text.replace(secret, "[PRIVATE]")
            text = re.sub(r"lmms_[A-Za-z0-9_-]+|lmmk_[A-Za-z0-9_-]+", "[CREDENTIAL]", text)
            path = self.evidence / (label + ".log")
            with path.open("a") as stream:
                stream.write(text)
        ok = all(value == "PASS" for value in self.results.values()) and not cleanup_errors
        if ok:
            if self.work:
                shutil.rmtree(self.work)
            if self.sockdir:
                shutil.rmtree(self.sockdir)
        self.save("commands.json", self.commands)
        self.save("events.json", self.events)
        self.save("results.json", {"cases": self.results, "overall": "PASS" if ok else "INCOMPLETE",
                  "cleanup_errors": cleanup_errors, "fixture_removed": ok,
                  "limits": "local new-install actual Core only; no upgrade, browser, production, release or S11 acceptance"})
        if hasattr(self, "cgroup"):
            self.save("resource-final.json", {name: (self.cgroup / name).read_text().strip() for name in
                                             ("memory.peak", "memory.events", "cpu.stat")})
        if self.lock:
            self.lock.close()
        return 0 if ok else 1

    def run(self):
        try:
            for self.case in CASES:
                print("START", self.case, flush=True)
                getattr(self, self.case)()
                self.results[self.case] = "PASS"
                print("PASS", self.case, flush=True)
        except Exception as error:
            resource_block = isinstance(error, BlockingIOError) or (
                isinstance(error, Failure) and str(error) in ("2GiB_admission_closed", "home_space_low"))
            self.results[self.case] = "BLOCKED" if resource_block else "FAIL"
            # Fixed Failure labels only; network/JSON exceptions can contain payloads.
            label = str(error) if isinstance(error, Failure) else type(error).__name__
            self.save("failure.json", {"case": self.case, "label": label})
            print("FAIL", self.case, label, flush=True)
        finally:
            code = self.finish()
        return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--core-sha256", required=True)
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--freeze", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    need(re.fullmatch(r"[a-f0-9]{64}", args.core_sha256), "expected_final_sha256_required")
    os.umask(0o077)
    return Runner(args).run()


if __name__ == "__main__":
    raise SystemExit(main())
