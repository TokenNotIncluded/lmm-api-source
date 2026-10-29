#!/usr/bin/env python3
"""Independent A02 acceptance against an already-built, non-test Core executable.

Run only after the integrator hands off the frozen source and matching binary:
  python3 -B email_process.py --core /absolute/lmm-core \
      --source /absolute/frozen-source --evidence /absolute/new-private-evidence

No builds, Go, production credentials, account fixtures, or manual worker calls.
Requires local PostgreSQL 18 tools on PATH and a non-root user with /home storage.
The source/binary association is supplied by the integrator, not proved by hashes.
Source and executable hashes are checked again after execution. Evidence is never
overwritten. HTTP bodies, SMTP DATA, cookies, credentials and raw process output
stay in memory; only redacted process logs and structural observations are saved.
Private generated key/config/database files are removed after owned processes exit.
The acceptance runner should apply space-first before execution; this script also
records bounded disk/residue checks before initializing its disposable cluster.
"""

import argparse
import email.policy
from email.parser import BytesParser
from email.utils import getaddresses
import hashlib
import http.client
import json
import os
from pathlib import Path
import queue
import re
import resource
import secrets
import shutil
import signal
import socket
import socketserver
import subprocess
import tempfile
import threading
import time
from urllib.parse import parse_qs, quote, urlencode, urlsplit


class Failure(Exception):
    """Messages must be fixed labels, never interpolate business data."""


def require(condition, label):
    if not condition:
        raise Failure(label)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def source_manifest(root):
    # Include untracked frozen inputs too. Never follow links into a live checkout.
    excluded = {".git", "target", "node_modules", ".venv", "__pycache__"}
    result = {}
    for directory, directories, files in os.walk(root, followlinks=False):
        directories[:] = sorted(d for d in directories if d not in excluded)
        for name in sorted(files + [d for d in directories if (Path(directory) / d).is_symlink()]):
            path = Path(directory) / name
            relative = str(path.relative_to(root))
            if path.is_symlink():
                raise Failure("frozen source contains a symlink; supply self-contained inputs")
            result[relative] = digest(path)
    return {"excluded_directories": sorted(excluded), "files": result}


class Sink(socketserver.ThreadingTCPServer):
    allow_reuse_address = False
    daemon_threads = False
    block_on_close = True

    def __init__(self):
        self.messages = queue.Queue()
        self.errors = queue.Queue()
        self.hold_recipient = None
        self.held = threading.Event()
        self.release = threading.Event()
        super().__init__(("127.0.0.1", 0), SMTP)
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    def handle_error(self, request, client_address):
        # socketserver's default traceback can contain SMTP/local business data.
        self.errors.put("smtp_handler_failed")

    def close(self):
        self.release.set()
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)
        require(not self.thread.is_alive(), "smtp_listener_did_not_stop")


class SMTP(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(20)
        recipients = []

        def reply(value):
            self.wfile.write(value + b"\r\n")
            self.wfile.flush()

        reply(b"220 localhost acceptance SMTP")
        while True:
            line = self.rfile.readline(8193)
            if not line:
                return
            require(len(line) <= 8192, "smtp_command_too_large")
            command = line.split(b" ", 1)[0].strip().upper()
            if command in (b"EHLO", b"HELO"):
                reply(b"250-localhost\r\n250 8BITMIME")
            elif command == b"MAIL":
                recipients = []
                reply(b"250 OK")
            elif command == b"RCPT":
                match = re.fullmatch(rb"RCPT TO:<([^<>\r\n]+)>\s*", line, re.I)
                require(match is not None, "smtp_invalid_recipient_envelope")
                recipients.append(match[1].decode("ascii"))
                reply(b"250 OK")
            elif command == b"DATA":
                require(bool(recipients), "smtp_data_without_recipient")
                reply(b"354 End with dot")
                data = bytearray()
                while True:
                    chunk = self.rfile.readline(65537)
                    require(bool(chunk), "smtp_incomplete_data")
                    if chunk == b".\r\n":
                        break
                    data.extend(chunk[1:] if chunk.startswith(b"..") else chunk)
                    require(len(data) <= 262144, "smtp_data_too_large")
                message = BytesParser(policy=email.policy.default).parsebytes(bytes(data))
                require(not message.defects, "smtp_mime_defects")
                # get_content decodes Content-Transfer-Encoding and charset.
                bodies = [part.get_content() for part in message.walk()
                          if part.get_content_type() == "text/plain" and not part.is_multipart()]
                require(bool(bodies), "smtp_plaintext_part_missing")
                to = [address for _, address in getaddresses(message.get_all("To", []))]
                require(to == recipients, "smtp_header_envelope_mismatch")
                self.server.messages.put((tuple(recipients), "\n".join(bodies)))
                if self.server.hold_recipient in recipients:
                    self.server.held.set()
                    require(self.server.release.wait(12), "smtp_hold_timed_out")
                reply(b"250 accepted")
            elif command == b"QUIT":
                reply(b"221 bye")
                return
            elif command in (b"NOOP", b"RSET"):
                reply(b"250 OK")
            else:
                reply(b"502 unsupported")


class Process:
    """Drain pipes immediately; redact only after all returned secrets are known."""

    def __init__(self, runner, label, argv, env, input_bytes=None):
        self.runner, self.label = runner, label
        self.output = bytearray()
        self.overflow = False
        self.record = {"label": label, "argv": list(map(str, argv)),
                       "cwd": str(runner.source), "exit_code": None}
        runner.commands.append(self.record)
        self.process = subprocess.Popen(list(map(str, argv)), cwd=runner.source, env=env,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, start_new_session=True)
        self.record["pid"] = self.process.pid
        self.started = time.monotonic()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        runner.processes.append(self)
        if input_bytes is not None:
            self.process.stdin.write(input_bytes)
        self.process.stdin.close()

    def read(self):
        while block := self.process.stdout.read(8192):
            if len(self.output) + len(block) <= 4 * 1024 * 1024:
                self.output.extend(block)
            else:
                self.overflow = True

    def wait(self, timeout=40):
        code = self.process.wait(timeout=timeout)
        self.reader.join(timeout=5)
        require(not self.reader.is_alive(), "process_log_pipe_not_closed")
        self.record.update(exit_code=code, elapsed_seconds=round(time.monotonic() - self.started, 3),
                           output_limit_exceeded=self.overflow)
        require(not self.overflow, "process_output_limit_exceeded")
        return code

    def signal(self, sig):
        if self.process.poll() is None:
            os.killpg(self.process.pid, sig)
            self.runner.events.append({"event": "process_signal", "label": self.label,
                                       "signal": sig.name})


class Runner:
    def __init__(self, args):
        self.core = args.core.resolve(strict=True)
        self.source = args.source.resolve(strict=True)
        self.evidence = args.evidence.absolute()
        require(not self.evidence.resolve().is_relative_to(self.source), "evidence_must_be_outside_frozen_source")
        self.evidence.mkdir(mode=0o700, parents=False, exist_ok=False)
        self.env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                    "HOME": str(Path.home()), "LANG": "C", "LC_ALL": "C", "TZ": "UTC"}
        self.commands, self.events, self.processes, self.secrets = [], [], [], set()
        self.verification_requests = {}
        self.failures, self.checks = [], {}
        self.work = self.socket_dir = self.sink = self.pg = None
        self.before = None
        self.port = None

    def save(self, name, value):
        (self.evidence / name).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")

    def remember(self, value):
        if isinstance(value, str) and value:
            self.secrets.add(value)
            self.secrets.add(quote(value, safe=""))
        return value

    def redact(self, raw):
        text = raw.decode("utf-8", errors="replace")
        for secret in sorted(self.secrets, key=len, reverse=True):
            text = text.replace(secret, "[PRIVATE]")
        # Also cover a delivery that failed before the runner learned its proof.
        text = re.sub(r"https?://[^\s<>\"]+", "[URL]", text)
        text = re.sub(r"[A-Za-z0-9_.+%-]+@[A-Za-z0-9.-]+", "[ADDRESS]", text)
        text = re.sub(r"(?i)\b[0-9a-f]{6}\b|\b[0-9a-f]{32,}\b", "[OPAQUE]", text)
        text = re.sub(r"(?i)(bearer|token|password|cookie|authorization)\s*[:= ]\s*\S+",
                      r"\1=[PRIVATE]", text)
        return text

    def spawn(self, label, argv, env=None, input_bytes=None):
        return Process(self, label, argv, self.env if env is None else env, input_bytes)

    def run(self, label, argv, env=None, input_bytes=None, timeout=40):
        process = self.spawn(label, argv, env, input_bytes)
        try:
            code = process.wait(timeout)
        except subprocess.TimeoutExpired:
            process.signal(signal.SIGTERM)
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                process.signal(signal.SIGKILL)
                process.wait(5)
            raise Failure(label + "_command_timeout") from None
        require(code == 0, label + "_command_failed")
        return bytes(process.output)

    def sql(self, label, statement, *, extra=()):
        # All SQL is setup, aggregate inspection or a single ACL mutation.
        # No business input or credentials are included in SQL or psql arguments.
        return self.run(label, ["psql", "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1", *extra],
                        self.pg_env, statement.encode()).decode().strip()

    def check(self, label, function):
        try:
            function()
            self.checks[label] = "PASS"
        except Exception as error:
            self.checks[label] = "FAIL"
            self.failures.append({"check": label, "reason": str(error) if isinstance(error, Failure)
                                  else type(error).__name__})

    def secret_file(self, name, value):
        path = self.work / name
        path.write_bytes(value if isinstance(value, bytes) else value.encode())
        path.chmod(0o600)
        return path

    def configuration(self, port):
        q = lambda path: json.dumps(str(path))
        origin = f"http://127.0.0.1:{port}"
        sidecar = self.secret_file(f"email-{port}.toml", f'''challenge_key_id = "proof-v1"
proof_key_file = {q(self.work / 'proof.key')}
rate_key_file = {q(self.work / 'rate.key')}
active_key_id = "mail-v1"
reset_origin = {q(origin)}
local_http = true
[key_files]
mail-v1 = {q(self.work / 'mail.key')}
[smtp]
host = "127.0.0.1"
port = {self.sink.server_address[1]}
from = "noreply@email.invalid"
tls = "plaintext_loopback_test"
''')
        config = self.secret_file(f"core-{port}.toml", f'''version = 1
[server]
listen = "127.0.0.1:{port}"
worker_threads = 2
drain_timeout_seconds = 10
[database]
url_file = {q(self.work / 'runtime.url')}
max_connections = 4
[identity]
registration_enabled = true
password_register_enabled = true
oauth_register_enabled = false
registration_disabled_methods = []
email_verification_enabled = true
email_domain_restriction_enabled = false
email_domain_whitelist = []
email_config_file = {q(sidecar)}
[identity.web]
origin = {q(origin)}
refresh_key_file = {q(self.work / 'refresh.key')}
local_http = true
user_agreement_enabled = true
privacy_policy_enabled = true
''')
        self.events.append({"event": "generated_config", "core_toml_sha256": digest(config),
                            "email_toml_sha256": digest(sidecar), "listen": f"127.0.0.1:{port}",
                            "local_http": True, "email_verification": True,
                            "agreement_required": True, "privacy_required": True,
                            "smtp_transport": "plaintext_loopback_test",
                            "independent_raw32_keys": True})
        return config

    def http(self, label, method, path, body=None, headers=None, port=None, quiet=False):
        require(path.startswith("/") and "?" not in path and "#" not in path,
                "request_target_must_not_contain_secrets")
        port = self.port if port is None else port
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        request_headers = {"Origin": f"http://127.0.0.1:{port}"}
        request_headers.update(headers or {})
        encoded = None if body is None else json.dumps(body).encode()
        if encoded is not None:
            request_headers["Content-Type"] = "application/json"
        try:
            connection.request(method, path, encoded, request_headers)
            response = connection.getresponse()
            raw = response.read(262145)
            require(len(raw) <= 262144, "http_response_too_large")
            value = json.loads(raw)
            status = response.status
            returned = {key.lower(): val for key, val in response.getheaders()}
            if "set-cookie" in returned:
                self.remember(returned["set-cookie"])
                self.remember(returned["set-cookie"].split(";", 1)[0])
            data = value.get("data")
            if isinstance(data, dict):
                self.remember(data.get("access_token"))
                if isinstance(data.get("session"), dict):
                    self.remember(data["session"].get("sid"))
            if not quiet:
                self.events.append({"event": "http", "label": label, "method": method,
                                    "path": path, "status": status,
                                    "success": value.get("success") is True,
                                    "no_store": returned.get("cache-control") == "no-store"})
            return status, returned, value
        finally:
            connection.close()

    def success(self, label, method, path, body=None, headers=None, status=200):
        result = self.http(label, method, path, body, headers)
        require(result[0] == status and result[2].get("success") is True, label + "_rejected")
        require(result[1].get("cache-control") == "no-store", label + "_cacheable")
        return result

    def denied(self, label, path, body=None, headers=None, method="POST", statuses=(400, 403)):
        status, _, value = self.http(label, method, path, body, headers)
        require(status in statuses and value.get("success") is False, label + "_not_rejected")

    def mail(self, recipient, kind):
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline:
            require(self.sink.errors.empty(), "smtp_sink_failed")
            try:
                recipients, body = self.sink.messages.get(timeout=0.2)
            except queue.Empty:
                continue
            if recipient not in recipients:
                continue
            if kind == "code":
                match = re.search(r"verification code is: ([0-9a-f]{6})\b", body)
                if match:
                    self.events.append({"event": "smtp_data_decoded", "kind": kind})
                    return self.remember(match[1])
            elif kind == "reset":
                links = re.findall(r"http://[^\s<>]+", body)
                if not links:
                    continue
                require(len(links) == 1, "reset_link_count")
                link = self.remember(links[0])
                parsed = urlsplit(link)
                require(parsed.scheme == "http" and parsed.netloc == f"127.0.0.1:{self.port}"
                        and parsed.path == "/user/reset" and not parsed.query,
                        "reset_link_not_bound_to_configured_origin_and_fragment")
                values = parse_qs(parsed.fragment, strict_parsing=True)
                require(set(values) == {"email", "token"} and values["email"] == [recipient]
                        and len(values["token"]) == 1, "reset_link_fields_invalid")
                token = values["token"][0]
                require(re.fullmatch(r"[0-9a-f]{32}", token), "reset_token_format")
                self.events.append({"event": "smtp_data_decoded", "kind": kind,
                                    "origin_bound": True, "proof_in_fragment": True})
                return self.remember(token)
        raise Failure("automatic_worker_smtp_delivery_timeout_" + kind)

    def startup(self, process, port):
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline:
            require(process.process.poll() is None, "formal_core_exited_before_ready")
            try:
                status, _, body = self.http("readiness", "GET", "/health/ready", port=port, quiet=True)
                if status == 200:
                    require(body.get("service") == "lmm-core" and body.get("status") == "ready"
                            and body.get("capabilities", {}).get("identity") is True
                            and body["capabilities"].get("legacy_web_auth") is True,
                            "formal_core_readiness_contract")
                    self.events.append({"event": "formal_core_ready", "pid": process.process.pid})
                    return
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(0.1)
        raise Failure("formal_core_readiness_timeout")

    def setup(self):
        require(os.geteuid() != 0, "run_as_non_root")
        require(Path.home().resolve().is_relative_to(Path("/home")), "home_storage_required")
        require(self.core.is_file() and os.access(self.core, os.X_OK), "executable_core_required")
        for tool in ("initdb", "postgres", "pg_isready", "psql"):
            require(shutil.which(tool) is not None, "missing_tool_" + tool)
        self.before = {"core_path": str(self.core), "core_sha256": digest(self.core),
                       "source_path": str(self.source), "source": source_manifest(self.source),
                       "harness_sha256": digest(Path(__file__).resolve()),
                       "association": "integrator supplied; hashes alone do not prove build provenance"}
        self.save("frozen-inputs.json", self.before)
        self.run("core_version", [self.core, "--version"])
        self.run("core_compiled_schema_manifest", [self.core, "schema-manifest"])
        self.run("disk_before", ["df", "-h", Path.home(), "/tmp", self.evidence])
        residue = sorted(Path.home().glob(".lmm-email-process-*"))
        self.save("residue-before.json", [str(path) for path in residue])
        for tool in ("dust", "prefixlens"):
            if shutil.which(tool):
                for index, path in enumerate([self.evidence, *residue]):
                    command = ([tool, "-d", "1", path] if tool == "dust"
                               else [tool, path, "--summary"])
                    try:
                        self.run(f"space_{tool}_{index}", command, timeout=15)
                    except (Failure, subprocess.TimeoutExpired):
                        self.events.append({"event": "space_scan_incomplete", "tool": tool})
            else:
                self.events.append({"event": "space_tool_unavailable", "tool": tool})
        require(shutil.disk_usage(Path.home()).free >= 512 * 1024 * 1024, "insufficient_home_space")
        self.work = Path(tempfile.mkdtemp(prefix=".lmm-email-process-", dir=Path.home()))
        self.socket_dir = Path(tempfile.mkdtemp(prefix="lmm-mail-", dir="/tmp"))
        self.work.chmod(0o700)
        self.socket_dir.chmod(0o700)
        self.admin = "mail_admin_" + secrets.token_hex(8)
        self.role = "mail_runtime_" + secrets.token_hex(8)
        self.db = "mail_acceptance"
        self.pg_env = {**self.env, "PGHOST": str(self.socket_dir), "PGPORT": "5432",
                       "PGUSER": self.admin, "PGDATABASE": self.db, "PGSSLMODE": "disable"}
        version = self.run("postgres_version", ["postgres", "--version"])
        require(re.search(rb"\b18\.", version), "postgres_18_required")
        self.run("initdb", ["initdb", "-D", self.work / "data", "-U", self.admin,
                             "--no-locale", "--encoding=UTF8", "--auth-local=trust",
                             "--auth-host=reject", "--wal-segsize=1"], timeout=60)
        self.pg = self.spawn("postgres", ["postgres", "-D", self.work / "data",
                             "-c", "listen_addresses=", "-c", f"unix_socket_directories={self.socket_dir}",
                             "-c", "unix_socket_permissions=0700", "-c", "shared_buffers=16MB",
                             "-c", "max_connections=24", "-c", "min_wal_size=2MB",
                             "-c", "max_wal_size=128MB", "-c", "log_statement=none",
                             "-c", "log_min_error_statement=panic",
                             "-c", "log_parameter_max_length_on_error=0"])
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            require(self.pg.process.poll() is None, "private_postgres_exited")
            probe = self.spawn("pg_ready", ["pg_isready", "-q", "-t", "1"],
                               {**self.pg_env, "PGDATABASE": "postgres"})
            if probe.wait(3) == 0:
                break
            time.sleep(0.1)
        else:
            raise Failure("private_postgres_not_ready")
        self.run("create_roles_database", ["psql", "-X", "-v", "ON_ERROR_STOP=1"],
                 {**self.pg_env, "PGDATABASE": "postgres"}, f'''
CREATE ROLE lmm_core_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE {self.role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
CREATE DATABASE {self.db} OWNER lmm_core_owner;
'''.encode())
        self.sql("installation_boundary", f'''
REVOKE CREATE,TEMPORARY ON DATABASE {self.db} FROM PUBLIC;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT CREATE ON DATABASE {self.db} TO lmm_core_owner;
''')
        def url(user, owner=False):
            params = {"host": str(self.socket_dir), "sslmode": "disable"}
            if owner:
                params["options"] = "-c role=lmm_core_owner"
            return f"postgresql://{user}@localhost:5432/{self.db}?{urlencode(params)}"
        owner_url = self.secret_file("owner.url", url(self.admin, True))
        self.secret_file("runtime.url", url(self.role))
        owner_config = self.secret_file("owner.toml", "version=1\n[database]\nurl_file="
                                        + json.dumps(str(owner_url)) + "\n")
        self.run("core_init_db", [self.core, "--config", owner_config, "init-db"], timeout=60)
        self.run("official_runtime_grants", ["psql", "-X", "-v", "ON_ERROR_STOP=1",
                 "-v", f"runtime_role={self.role}", "-v", "enable_web_identity=true",
                 "-f", self.source / "deployment/postgres/grant-core-runtime.sql"], self.pg_env)
        require(self.sql("identity_version", "SELECT version FROM core_meta.schema_contract WHERE singleton;") == "6",
                "identity_v6_required")
        require(self.sql("no_pg_tcp", "SHOW listen_addresses;") == "", "postgres_tcp_enabled")
        require(self.sql("runtime_role_boundary", f'''
SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)
 AND NOT pg_has_role(rolname,'lmm_core_owner','MEMBER')
 FROM pg_roles WHERE rolname='{self.role}';
''') == "t", "runtime_role_is_privileged")
        require(self.sql("object_owner", """
SELECT bool_and(pg_get_userbyid(c.relowner)='lmm_core_owner')
FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core_%';
""") == "t", "installation_objects_not_owner_owned")
        for name in ("proof", "rate", "mail", "refresh"):
            self.secret_file(name + ".key", secrets.token_bytes(32))
        self.sink = Sink()
        self.port = free_port()
        self.config = self.configuration(self.port)
        self.main = self.spawn("core_main", [self.core, "--config", self.config, "serve"])
        self.startup(self.main, self.port)

    def status(self):
        _, _, body = self.success("public_status", "GET", "/api/status")
        data = body["data"]
        for field in ("email_verification", "register_enabled", "password_register_enabled",
                      "password_login_enabled", "user_agreement_enabled", "privacy_policy_enabled"):
            require(data.get(field) is True, "public_status_flag_" + field)
        for field in ("external_model_providers_enabled", "models_local_synthetic", "payments_enabled"):
            require(data.get(field) is False, "unexpected_upstream_capability_" + field)

    def registration_body(self, recipient):
        requested_at = time.monotonic()
        self.success("request_verification", "POST", "/api/identity/email/verification/request",
                     {"email": recipient, "purpose": "register"}, status=202)
        completed_at = time.monotonic()
        self.verification_requests.setdefault(recipient, (requested_at, completed_at))
        self.events.append({"event": "verification_request_timing",
                            "started_monotonic": requested_at,
                            "completed_monotonic": completed_at})
        return {"username": "mail_" + secrets.token_hex(7),
                "password": self.remember("A02!" + secrets.token_hex(8)), "email": recipient,
                "verification_code": self.mail(recipient, "code"), "accepted_legal": True}

    def wait_issue_window(self, recipient):
        requested_at, completed_at = self.verification_requests[recipient]
        # The persisted issue precedes HTTP completion. Using completion as the
        # upper bound also covers time spent inside the original HTTP request.
        deadline = completed_at + 60 + 1
        timing = {"first_request_started_monotonic": requested_at,
                  "first_request_completed_monotonic": completed_at,
                  "window_seconds": 60, "margin_seconds": 1,
                  "deadline_monotonic": deadline, "segments": []}
        self.save("issue-window.json", timing)
        while True:
            before = time.monotonic()
            remaining = deadline - before
            if remaining <= 0:
                break
            time.sleep(min(5, remaining))
            after = time.monotonic()
            timing["segments"].append({"started_monotonic": before,
                                       "ended_monotonic": after,
                                       "slept_seconds": after - before,
                                       "remaining_seconds": max(0, deadline - after)})
            self.save("issue-window.json", timing)
        timing["finished_monotonic"] = time.monotonic()
        timing["elapsed_since_first_request_seconds"] = timing["finished_monotonic"] - requested_at
        self.save("issue-window.json", timing)
        self.events.append({"event": "real_issue_window_elapsed", **timing})

    def login(self, registration, password):
        _, headers, body = self.success("web_login", "POST", "/api/user/login",
                                       {"username": registration["username"], "password": password})
        bundle = body["data"]
        require(bundle["user"]["email"] == registration["email"], "login_email_not_registered")
        require(bundle.get("token_type") == "Bearer", "login_token_type")
        cookie = headers.get("set-cookie", "").split(";", 1)[0]
        require(cookie.startswith("new_api_refresh="), "refresh_cookie_missing")
        return {"Authorization": "Bearer " + bundle["access_token"],
                "X-Auth-Session": bundle["session"]["sid"]}, cookie

    def business(self):
        recipient = self.remember("user-" + secrets.token_hex(10) + "@example.invalid")
        registration = self.registration_body(recipient)
        self.denied("web_legal_required", "/api/user/register", {**registration, "accepted_legal": False})
        self.success("web_register", "POST", "/api/user/register", registration)
        self.denied("registration_proof_replay", "/api/user/register",
                    {**registration, "username": "replay_" + secrets.token_hex(6)})
        auth, cookie = self.login(registration, registration["password"])
        self.success("session_before_reset", "GET", "/api/user/self", headers=auth)
        self.wait_issue_window(recipient)
        self.success("request_reset", "POST", "/api/identity/email/password-reset/request",
                     {"email": recipient}, status=202)
        token = self.mail(recipient, "reset")
        new_password = self.remember("New!" + secrets.token_hex(8))
        reset = {"email": recipient, "token": token, "new_password": new_password}
        self.success("consume_reset", "POST", "/api/user/reset", reset)
        self.denied("old_session_revoked", "/api/user/self", headers=auth, method="GET", statuses=(401,))
        self.denied("old_refresh_revoked", "/api/user/auth/refresh",
                    headers={"Cookie": cookie, "X-Auth-Session": auth["X-Auth-Session"]}, statuses=(401,))
        self.denied("old_password_rejected", "/api/user/login",
                    {"username": registration["username"], "password": registration["password"]}, statuses=(401,))
        new_auth, _ = self.login(registration, new_password)
        self.success("new_session_valid", "GET", "/api/user/self", headers=new_auth)
        self.denied("reset_proof_replay", "/api/user/reset",
                    {**reset, "new_password": self.remember("Replay!" + secrets.token_hex(6))})
        self.login(registration, new_password)
        self.success("replay_did_not_revoke_new_session", "GET", "/api/user/self", headers=new_auth)

    def native_boundary(self):
        recipient = self.remember("native-" + secrets.token_hex(10) + "@example.invalid")
        registration = self.registration_body(recipient)
        # Valid unused proof and explicit consent isolate the native enable gate.
        # A malformed/expired proof or missing consent would be a false positive.
        self.denied("native_email_register_disabled", "/core/v1/identity/email/register", registration)
        self.success("native_denial_preserved_valid_proof", "POST", "/api/user/register", registration)
        self.login(registration, registration["password"])

    def worker_shutdown(self):
        recipient = self.remember("drain-" + secrets.token_hex(10) + "@example.invalid")
        self.sink.hold_recipient = recipient
        self.success("request_drain_challenge", "POST", "/api/identity/email/verification/request",
                     {"email": recipient, "purpose": "register"}, status=202)
        self.mail(recipient, "code")
        require(self.sink.held.wait(3), "worker_not_in_smtp_data")
        # Read only the random outbox ID, never its encrypted/clear business data.
        row = self.sql("inflight_worker_row", """
SELECT id FROM core_identity.email_delivery_outbox WHERE status='leased' AND smtp_started_at IS NOT NULL;
""")
        require(re.fullmatch(r"[0-9a-f]{32}", row), "expected_one_inflight_worker")
        self.main.signal(signal.SIGTERM)
        try:
            time.sleep(1)
            require(self.main.process.poll() is None, "core_exited_before_worker_completed")
        finally:
            self.sink.release.set()
        require(self.main.wait(35) == 0, "core_graceful_shutdown_failed")
        require(self.sql("worker_finalized_before_exit", f"""
SELECT status='accepted_by_smtp' AND finished_at IS NOT NULL AND lease_token IS NULL
FROM core_identity.email_delivery_outbox WHERE id='{row}';
""") == "t", "worker_delivery_not_durable_before_exit")

    def missing_permission(self):
        # First main must have stopped: the second process gets a distinct port,
        # so bind conflicts cannot masquerade as the requested startup rejection.
        require(self.main.process.poll() == 0, "positive_core_not_cleanly_stopped")
        self.sql("revoke_required_email_permission",
                 f"REVOKE INSERT ON core_identity.email_delivery_outbox FROM {self.role};")
        require(self.sql("verify_revoked_permission", f"SELECT has_table_privilege('{self.role}',"
                         "'core_identity.email_delivery_outbox','INSERT');") == "f", "email_acl_not_revoked")
        port = free_port()
        config = self.configuration(port)
        candidate = self.spawn("core_missing_email_acl", [self.core, "--config", config, "serve"])
        deadline = time.monotonic() + 20
        listened = False
        while candidate.process.poll() is None and time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    listened = True
            except OSError:
                pass
            time.sleep(0.02)
        require(candidate.process.poll() is not None, "missing_email_acl_did_not_refuse_startup")
        require(candidate.wait() not in (0, -signal.SIGTERM, -signal.SIGKILL) and not listened,
                "missing_email_acl_not_fail_closed")
        require(b"MissingPrivileges" in candidate.output
                or b"core runtime database authority is incomplete" in candidate.output,
                "startup_rejection_not_attributed_to_runtime_permissions")
        # Restore exactly the removed grant and prove the same config starts.
        self.sql("restore_required_email_permission",
                 f"GRANT INSERT ON core_identity.email_delivery_outbox TO {self.role};")
        restored = self.spawn("core_acl_restored", [self.core, "--config", config, "serve"])
        self.startup(restored, port)
        restored.signal(signal.SIGTERM)
        require(restored.wait(35) == 0, "restored_core_shutdown_failed")

    def cleanup(self):
        if self.sink:
            self.sink.release.set()
        # Stop Core/commands before PG; SIGKILL is recorded as cleanup failure.
        for process in [p for p in self.processes if p is not self.pg] + ([self.pg] if self.pg else []):
            try:
                process.signal(signal.SIGINT if process is self.pg else signal.SIGTERM)
                process.wait(40)
            except Exception:
                self.failures.append({"check": "cleanup", "reason": "process_required_forced_cleanup",
                                      "label": process.label})
                process.signal(signal.SIGKILL)
                try:
                    process.wait(5)
                except Exception:
                    pass
        if self.sink:
            try:
                self.sink.close()
                require(self.sink.errors.empty(), "smtp_handler_failed")
            except Exception:
                self.failures.append({"check": "cleanup", "reason": "smtp_cleanup_failed"})
        exited = all(p.process.poll() is not None for p in self.processes)
        removed = []
        if exited:
            for path in (self.work, self.socket_dir):
                if path is not None:
                    try:
                        shutil.rmtree(path)
                        removed.append({"path": str(path), "removed": not path.exists()})
                    except OSError:
                        self.failures.append({"check": "cleanup", "reason": "private_fixture_removal_failed"})
        else:
            self.failures.append({"check": "cleanup", "reason": "owned_process_still_running"})
        self.save("cleanup.json", {"all_owned_processes_exited": exited, "directories": removed,
                                   "home_free_bytes_after": shutil.disk_usage(Path.home()).free})

    def execute(self):
        try:
            self.setup()
            self.checks["formal_main_identity_v6"] = "PASS"
            self.check("public_status", self.status)
            self.check("web_registration_login_reset_replay", self.business)
            self.check("native_registration_boundary", self.native_boundary)
            self.check("worker_shutdown", self.worker_shutdown)
            self.check("missing_email_runtime_permission", self.missing_permission)
        except (Exception, KeyboardInterrupt) as error:
            self.failures.append({"check": "setup_or_interruption", "reason": str(error)
                                  if isinstance(error, Failure) else type(error).__name__})
        finally:
            self.cleanup()
            if self.before is not None:
                try:
                    after = {"core_sha256": digest(self.core), "source": source_manifest(self.source),
                             "harness_sha256": digest(Path(__file__).resolve())}
                    self.save("frozen-inputs-after.json", after)
                    require(all(after[key] == self.before[key] for key in after), "frozen_inputs_changed")
                except Exception:
                    self.failures.append({"check": "frozen_inputs", "reason": "inputs_changed_or_unreadable"})
            for index, process in enumerate(self.processes):
                name = f"{index:03d}-{process.label}.log"
                (self.evidence / name).write_text(self.redact(bytes(process.output)))
                process.record["redacted_log"] = name
            self.save("commands.json", self.commands)
            self.save("observations.json", self.events)
            for label in ("formal_main_identity_v6", "public_status", "web_registration_login_reset_replay",
                          "native_registration_boundary", "worker_shutdown", "missing_email_runtime_permission"):
                self.checks.setdefault(label, "NOT_RUN")
            passed = not self.failures and all(value == "PASS" for value in self.checks.values())
            self.save("results.json", {"result": "PASS" if passed else "FAIL", "exit_code": 0 if passed else 1,
                       "checks": self.checks, "failures": self.failures,
                       "scope": "formal Core process, local private PostgreSQL and actual SMTP delivery",
                       "limits": ["no browser rendering acceptance", "no production or paid upstream",
                                  "binary-source provenance supplied by integrator", "logs are redacted"]})
        return 0 if passed else 1


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    # Catch ordinary termination to release the blocked SMTP transaction and reap
    # owned children. SIGKILL/host failure cannot be cleaned up by any runner.
    def interrupted(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        runner = Runner(args)
        return runner.execute()
    except (Exception, KeyboardInterrupt) as error:
        print("email_process: setup failed (" + type(error).__name__ + "); inspect private evidence")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
