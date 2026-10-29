#!/usr/bin/env python3
"""Fixture adapted from scripts/test-upgrade-offline.py r4 (SHA 5ec5eb93...).

Development verification, not production cutover or whole-site acceptance.
All clusters, units, credentials and faults belong to this invocation.
"""
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
        module = Path(__file__).resolve().parents[1] / "console/independent_process/pg_fault.py"
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
        receipt = json.loads(self.run("previous-native-first-owner", [self.args.previous_core, "--config", owner, "create-first-owner"],
                  json.dumps({"username": "offlineowner", "display_name": "Synthetic owner", "password": "fixture-" + secrets.token_hex(16)}).encode()).stdout)
        need(receipt.get("setup_completed") is True, "first_owner_setup")
        self.run("canonical-grants", ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-h", self.target["socket"],
                 "-p", self.target["port"], "-U", "postgres", "-d", "coretarget", "-v", "enable_console=true", "-f",
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
