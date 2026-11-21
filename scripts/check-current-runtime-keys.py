#!/usr/bin/env python3
"""Run the used-target key regression on an exact, disposable Core78 fixture."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
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
import sys
import tarfile
import tempfile
import time
from urllib.parse import urlencode

REVISION = "fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5"
CORE_SHA = "62df884eff7e41e8e67aed070a34409d1d4329f8a0f3952c08925d3a83ca9877"
MANIFEST_SHA = "b9d0a472bddc826e0f6bfa0e725e409e67cbaabad6b0ad80cd512de24da74c2b"
CONTROL = Path(__file__).resolve().parents[1]
os.umask(0o077)
sys.dont_write_bytecode = True


def need(ok, message):
    if not ok:
        raise RuntimeError(message)


def sha(path):
    with Path(path).open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def save(path, value):
    with Path(path).open("x") as f:
        json.dump(value, f, sort_keys=True, indent=2)
        f.write("\n")


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


def run(argv, output, label, *, data=None, env=None, timeout=120):
    result = subprocess.run(list(map(str, argv)), input=data, capture_output=True,
                            env=env, timeout=timeout)
    (output / (label + ".stdout.log")).write_bytes(result.stdout)
    (output / (label + ".stderr.log")).write_bytes(result.stderr)
    save(output / (label + ".command.json"), {
        "argv": list(map(str, argv)), "exit": result.returncode,
        "stdout_sha256": hashlib.sha256(result.stdout).hexdigest(),
        "stderr_sha256": hashlib.sha256(result.stderr).hexdigest()})
    need(result.returncode == 0, "command failed: " + label)
    return result.stdout


def source_check(source):
    need(subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"],
                                 text=True).strip() == REVISION, "wrong product source")
    need(not subprocess.check_output(["git", "-C", str(source), "status", "--porcelain"],
                                     text=True).strip(), "product source is dirty")


def runtime_probe(source, output, work, sock, db, port, runtime, env, sql):
    """Exercise native auth, recovery and quota projection with synthetic keys."""
    probe = json.loads((work / "regression/runtime-probes.private.json").read_text())
    need(probe["database"] == db and probe["expected_core78_manifest_sha256"] == MANIFEST_SHA,
         "runtime probe belongs to another fixture")
    sql(f"ALTER ROLE {runtime} LOGIN; GRANT CONNECT ON DATABASE {db} TO {runtime};", admin=True)
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    http_port = listener.getsockname()[1]
    listener.close()
    origin = "http://127.0.0.1:" + str(http_port)
    runtime_url = f"postgresql://{runtime}@localhost:{port}/{db}?" + urlencode({
        "host": str(sock), "sslmode": "disable"})
    (work / "runtime.url").write_text(runtime_url + "\n")
    (work / "refresh.key").write_bytes(secrets.token_bytes(32))
    (work / "api-key.key").write_bytes(bytes.fromhex(probe["encryption_key_hex"]))
    config = f'''version=1
[server]
listen="127.0.0.1:{http_port}"
worker_threads=1
drain_timeout_seconds=10
[database]
url_file={json.dumps(str(work / "runtime.url"))}
max_connections=4
[console]
enabled=true
[identity]
api_key_encryption_key_file={json.dumps(str(work / "api-key.key"))}
registration_enabled=false
password_register_enabled=false
oauth_register_enabled=false
registration_disabled_methods=[]
email_verification_enabled=false
email_domain_restriction_enabled=false
email_domain_whitelist=[]
[identity.web]
origin={json.dumps(origin)}
refresh_key_file={json.dumps(str(work / "refresh.key"))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
'''
    (work / "runtime.toml").write_text(config)
    log = (work / "runtime.log").open("xb")
    proc = subprocess.Popen([str(output / "lmm-core"), "--config", str(work / "runtime.toml"), "serve"],
                            stdout=log, stderr=subprocess.STDOUT, env=env)
    events = []

    def request(method, path, bearer=None):
        headers = {"Origin": origin}
        if bearer is not None:
            headers["Authorization"] = "Bearer " + bearer
        client = http.client.HTTPConnection("127.0.0.1", http_port, timeout=5)
        try:
            client.request(method, path, headers=headers)
            response = client.getresponse()
            raw = response.read(1024 * 1024)
            value = json.loads(raw)
            return response.status, dict(response.getheaders()), value
        finally:
            client.close()

    try:
        for _ in range(200):
            need(proc.poll() is None, "candidate Core exited before readiness")
            try:
                status, _, value = request("GET", "/health/ready")
                if status == 200:
                    break
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError("candidate Core readiness timeout")
        need(sha(Path(f"/proc/{proc.pid}/exe")) == CORE_SHA, "running candidate executable differs")
        session = probe["session"]
        status, _, value = request("GET", "/core/v1/identity", session["bearer"])
        need(status == 200 and value["credential_id"] == session["credential_id"]
             and value["user_id"] == probe["user_id"], "preexisting valid Session no longer authenticates")
        events.append({"case": "preexisting_session_auth", "status": status})
        for key in probe["keys"]:
            status, _, value = request("GET", "/core/v1/identity", key["bearer"])
            expected = 200 if key["expected_auth"] == "allowed" else 401
            need(status == expected, "imported key auth status differs")
            if expected == 200:
                need(value["credential_id"] == key["credential_id"] and value["user_id"] == probe["user_id"],
                     "imported key authenticated as the wrong identity")
            events.append({"case": "key_auth", "credential_id": key["credential_id"], "status": status})
            status, _, value = request("GET", "/api/token/" + str(key["credential_id"]), session["bearer"])
            need(status == 200 and value.get("success") is True, "native key projection failed")
            view = value["data"]
            need(view["used_quota"] == key["used_units"] and view["remain_quota"] == key["remaining_units"]
                 and view["unlimited_quota"] == key["unlimited"], "historical key quota was reset or re-scaled")
            status, headers, value = request("POST", "/api/token/" + str(key["credential_id"]) + "/key", session["bearer"])
            if key["recoverable"]:
                need(status == 200 and value.get("success") is True and value["data"]["key"] == key["bearer"]
                     and "no-store" in next((v for k, v in headers.items() if k.lower() == "cache-control"), ""),
                     "key recovery is not bound to the new native ID or cache policy")
            else:
                need(status == 403, "one-time key unexpectedly has a recovery copy")
            events.append({"case": "quota_and_recovery", "credential_id": key["credential_id"],
                           "used_units": view["used_quota"], "remaining_units": view["remain_quota"],
                           "unlimited": view["unlimited_quota"], "recovery_status": status})
        save(work / "runtime-result.public.json", {"status": "PASS", "events": events,
             "actual_core_sha256": CORE_SHA, "real_provider_calls": 0,
             "scope": "preserved Session, imported key auth/recovery and historical quota projection",
             "model_admission_budget_tests": "owned by the same-candidate full CI and runtime suite"})
        return events
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=10)
                raise RuntimeError("candidate Core required forced termination")
        log.close()
        need(proc.returncode == 0, "candidate Core did not stop cleanly")


def password_runtime_probe(output, work, sock, db, port, runtime, env, sql, driver, testenv):
    """Start the fixed Core only after one acknowledged synthetic identity commit."""
    prepared = json.loads((work / "regression/result.public.json").read_text())
    need(prepared["status"] == "PREPARED_RUNTIME_NOT_RUN" and prepared["identity_commits"] == 1
         and prepared["synthetic_only"] is True and prepared["production_access"] is False,
         "password runtime cannot start before the isolated identity commit")
    sql(f"ALTER ROLE {runtime} LOGIN; GRANT CONNECT ON DATABASE {db} TO {runtime};", admin=True)
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    http_port = listener.getsockname()[1]
    listener.close()
    origin = "http://127.0.0.1:" + str(http_port)
    runtime_url = f"postgresql://{runtime}@localhost:{port}/{db}?" + urlencode({
        "host": str(sock), "sslmode": "disable"})
    (work / "runtime.url").write_text(runtime_url + "\n")
    (work / "refresh.key").write_bytes(secrets.token_bytes(32))
    (work / "api-key.key").write_bytes(secrets.token_bytes(32))
    config = f'''version=1
[server]
listen="127.0.0.1:{http_port}"
worker_threads=1
drain_timeout_seconds=10
[database]
url_file={json.dumps(str(work / "runtime.url"))}
max_connections=4
[console]
enabled=true
[identity]
api_key_encryption_key_file={json.dumps(str(work / "api-key.key"))}
registration_enabled=false
password_register_enabled=false
oauth_register_enabled=false
registration_disabled_methods=[]
email_verification_enabled=false
email_domain_restriction_enabled=false
email_domain_whitelist=[]
[identity.web]
origin={json.dumps(origin)}
refresh_key_file={json.dumps(str(work / "refresh.key"))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
'''
    (work / "runtime.toml").write_text(config)
    log_path = work / "password-runtime-core.log"
    log = log_path.open("xb")
    started_utc = datetime.now(timezone.utc).isoformat()
    proc = subprocess.Popen([str(output / "lmm-core"), "--config", str(work / "runtime.toml"), "serve"],
                            stdout=log, stderr=subprocess.STDOUT, env=env)
    lifecycle = {"status": "STARTED", "pid": proc.pid, "origin": origin,
                 "expected_core_sha256": CORE_SHA, "forced_termination": False,
                 "production_access": False, "started_utc": started_utc}
    try:
        for _ in range(200):
            need(proc.poll() is None, "fixed Core exited before password runtime readiness")
            connection = http.client.HTTPConnection("127.0.0.1", http_port, timeout=2)
            try:
                connection.request("GET", "/health/ready", headers={"Origin": origin})
                response = connection.getresponse()
                response.read(65536)
                if response.status == 200:
                    break
            except (OSError, http.client.HTTPException):
                pass
            finally:
                connection.close()
            time.sleep(0.1)
        else:
            raise RuntimeError("fixed Core password runtime readiness timeout")
        lifecycle["actual_core_sha256"] = sha(Path(f"/proc/{proc.pid}/exe"))
        need(lifecycle["actual_core_sha256"] == CORE_SHA, "running password Core differs")
        run([sys.executable, "-B", driver, "verify", "--fixture", work / "fixture.private.json",
             "--probes", work / "regression/runtime-probes.private.json", "--origin", origin,
             "--core-pid", str(proc.pid), "--expected-core-sha256", CORE_SHA,
             "--core-started-utc", started_utc,
             "--output", work / "password-runtime"], work, "identity-password-http", env=testenv, timeout=180)
        verified = json.loads((work / "password-runtime/result.public.json").read_text())
        need(verified["status"] == "PASS" and verified["password_runtime_login"] == "PASS"
             and verified["actual_core_sha256"] == CORE_SHA and verified["financial_state_unchanged"] is True
             and verified["production_authorized"] is False and verified["real_google_login"] == "NOT_RUN",
             "password runtime result did not establish its bounded scope")
        lifecycle["status"] = "VERIFIED"
        return verified
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                lifecycle["forced_termination"] = True
                proc.kill()
                proc.wait(timeout=10)
        log.close()
        lifecycle.update(core_exit=proc.returncode, core_process_stopped=proc.poll() is not None,
                         core_log_sha256=sha(log_path), finished_utc=datetime.now(timezone.utc).isoformat())
        try:
            state_path = work / "password-runtime/post-http-state.private.json"
            if state_path.exists():
                before_stop = json.loads(state_path.read_text())
                query = (work / "password-runtime/post-http-state-readback.sql").read_text()
                after_stop = json.loads(sql("BEGIN READ ONLY; " + query.rstrip(";\n") + "; COMMIT;"))
                probe = json.loads((work / "regression/runtime-probes.private.json").read_text())
                actual_heartbeat = after_stop["tables"].pop("core_operations.instances")
                running_heartbeat = before_stop["tables"].pop("core_operations.instances")
                lifecycle["heartbeat_rows_running"] = running_heartbeat["rows"]
                lifecycle["heartbeat_rows_after_stop"] = actual_heartbeat["rows"]
                lifecycle["all_other_application_state_unchanged_after_stop"] = before_stop == after_stop
                lifecycle["after_stop_non_heartbeat_state_sha256"] = hashlib.sha256(
                    json.dumps(after_stop, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
                need(running_heartbeat["rows"] == 1 and actual_heartbeat == probe["before_http_heartbeat"]
                     and actual_heartbeat["rows"] == 0 and before_stop == after_stop,
                     "Core shutdown did not preserve application state and remove only its heartbeat")
        finally:
            save(work / "password-runtime-lifecycle.public.json", lifecycle)
        need(proc.returncode == 0 and lifecycle["forced_termination"] is False,
             "password Core did not stop cleanly")


def acquire(source, output):
    source_check(source)
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    output.mkdir(mode=0o700)
    original = module("original_runtime_controller", CONTROL / "scripts/final-runtime-acceptance.py")
    artifact, size, zip_sha, name, tar_sha = original.COMPONENTS["core"]
    archive = original.download_artifact(output / "download", artifact, size, zip_sha,
        REVISION, original.DISTRIBUTION_RUN, "lmm-preview-core-" + REVISION)
    original.unzip(archive, output / "zip", {name, name + ".sha256"})
    package = output / "zip" / name
    need(sha(package) == tar_sha, "Core tar differs")
    distribution = module("fixed_distribution", source / "scripts/build-distribution.py")
    manifest = distribution.verify(package, REVISION)
    need(manifest["component"] == "core", "not a Core distribution")
    binary = output / "lmm-core"
    with tarfile.open(package, "r:gz") as tar:
        with tar.extractfile("bin/lmm-core") as f, binary.open("xb") as out:
            shutil.copyfileobj(f, out)
    binary.chmod(0o500)
    need(sha(binary) == CORE_SHA, "Core executable differs")
    save(output / "acquisition.json", {"revision": REVISION, "binary_sha256": CORE_SHA,
         "archive_sha256": tar_sha, "artifact_id": artifact, "product_rebuilt": False})


def execute(source, output, *, regression="keys"):
    need(regression in ("keys", "wallet", "identity", "identity-diagnostic", "identity-password", "key-seam", "quota-redemptions"), "unknown synthetic regression")
    source_check(source)
    need(not os.environ.get("GH_TOKEN") and not os.environ.get("GITHUB_TOKEN"),
         "artifact token reached database regression")
    need(os.geteuid() != 0 and sha(output / "lmm-core") == CORE_SHA, "invalid runner or binary")
    work = output / "fixture"
    work.mkdir(mode=0o700)
    sock = Path(tempfile.mkdtemp(prefix="lmm-keys-", dir="/tmp"))
    data = work / "data"
    nonce = secrets.token_hex(32)
    db = "lmm_runtime_keys_test_" + nonce[:12]
    port = "55474"
    owner, installer, runtime = "lmm_core_owner", "keys_installer", "keys_runtime"
    env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
    env.update(LC_ALL="C", PYTHONDONTWRITEBYTECODE="1", PGSSLMODE="disable", PGGSSENCMODE="disable")
    base = ["psql", "-X", "-qAt", "--no-password", "-v", "ON_ERROR_STOP=1",
            "-h", str(sock), "-p", port]
    counter = 0

    def sql(statement, *, admin=False, database=None):
        nonlocal counter
        counter += 1
        settings = dict(env)
        if not admin:
            settings["PGOPTIONS"] = "-c role=" + owner
        return run([*base, "-U", "fixture_admin" if admin else installer,
                    "-d", database or db], work, "sql-" + str(counter),
                   data=statement.encode(), env=settings)

    pg = None
    result = {"status": "FAIL", "regression": regression, "product_revision": REVISION, "core_sha256": CORE_SHA,
              "production_access": False, "product_rebuilt": False,
              "started_utc": datetime.now(timezone.utc).isoformat()}
    try:
        version = run(["postgres", "--version"], work, "postgres-version", env=env)
        need(b" 18." in version, "PostgreSQL18 is required")
        run(["initdb", "-D", data, "-U", "fixture_admin", "--no-locale", "--encoding=UTF8",
             "--auth-local=trust", "--auth-host=reject", "--wal-segsize=1"],
            work, "initdb", env=env)
        pglog = (work / "postgres.log").open("xb")
        pg = subprocess.Popen(["postgres", "-D", str(data), "-p", port,
            "-c", "listen_addresses=", "-c", "unix_socket_directories=" + str(sock),
            "-c", "unix_socket_permissions=0700", "-c", "shared_buffers=32MB",
            "-c", "max_connections=24", "-c", "autovacuum=off",
            "-c", "log_statement=none", "-c", "log_min_error_statement=panic",
            "-c", "log_parameter_max_length_on_error=0",
            "-c", "lmm.migration_instance_nonce=" + nonce],
            stdout=pglog, stderr=subprocess.STDOUT, env=env)
        for _ in range(100):
            need(pg.poll() is None, "isolated PostgreSQL exited")
            if (sock / (".s.PGSQL." + port)).exists() and subprocess.run(
                ["pg_isready", "-q", "-h", str(sock), "-p", port, "-U", "fixture_admin"],
                env=env).returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("isolated PostgreSQL readiness timeout")
        sql(f"CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT; "
            f"CREATE ROLE {runtime} LOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; "
            f"CREATE DATABASE {db} OWNER {owner}; REVOKE ALL ON DATABASE {db} FROM PUBLIC; "
            f"GRANT CONNECT ON DATABASE {db} TO {installer}; "
            f"COMMENT ON DATABASE {db} IS 'lmm-runtime-keys-synthetic:{nonce}';", admin=True, database="postgres")
        sql(f"GRANT EXECUTE ON FUNCTION pg_control_system() TO {owner}; "
            f"GRANT pg_read_all_settings TO {owner}; "
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC;", admin=True)
        url = f"postgresql://{installer}@localhost:{port}/{db}?" + urlencode({
            "host": str(sock), "sslmode": "disable", "options": "-c role=" + owner})
        (work / "owner.url").write_text(url + "\n")
        (work / "owner.toml").write_text('version=1\n[database]\nurl_file=' + json.dumps(str(work / "owner.url")) + "\n")
        run([output / "lmm-core", "--config", work / "owner.toml", "init-db"], work, "core-init", env=env)
        manifest = run([output / "lmm-core", "schema-manifest"], work, "compiled-manifest", env=env)
        need(hashlib.sha256(manifest).hexdigest() == MANIFEST_SHA, "compiled Core78 manifest differs")
        (work / "core78-manifest.json").write_bytes(manifest)
        seed = CONTROL / ("tests/current-key-production/seed.sql" if regression == "key-seam" else "tests/current-runtime-keys/seed.sql")
        seed_text = seed.read_text()
        if regression == "key-seam":
            # Select one numeric loopback destination before the immutable
            # publication and every migration preservation baseline exist.
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                provider_port = reservation.getsockname()[1]
            need(seed_text.count("KEY_SEAM_PROVIDER_PORT") == 1, "missing synthetic provider binding")
            seed_text = seed_text.replace("KEY_SEAM_PROVIDER_PORT", str(provider_port))
            save(work / "key-seam-provider.private.json", {"port": provider_port})
        sql(seed_text)
        run([*base, "-U", installer, "-d", db, "-v", "runtime_role=" + runtime,
             "-v", "enable_console=true", "-v", "enable_web_identity=true",
             "-v", "enable_models=true", "-f", source / "deployment/postgres/grant-core-runtime.sql"],
            work, "official-runtime-grants", env={**env, "PGOPTIONS": "-c role=" + owner})
        sql(f"ALTER ROLE {runtime} NOLOGIN;", admin=True)
        # The shared connection contract requires an owned private pgpass file
        # even for a local trust socket. This file contains no credentials.
        pgpass = work / "pgpass.private"
        with pgpass.open("x") as stream:
            stream.write("# Synthetic Unix-only trust fixture; no password is used.\n")
        pgpass.chmod(0o600)
        fixture = {"format_version": 1, "scope": "current-runtime-keys-synthetic-fixture",
            "connection": {"transport": "unix", "socket_directory": str(sock), "port": int(port),
                "database": db, "role": owner, "login_role": installer, "set_role": owner,
                "pgpass_file": str(pgpass)},
            "fixture_nonce": nonce, "core78_manifest": str(work / "core78-manifest.json"),
            "runtime_roles": [runtime], "fixture_user_id": 1, "fixture_group": "paid" if regression == "key-seam" else "default"}
        save(work / "fixture.private.json", fixture)
        keys = CONTROL / "tests/current-runtime-keys"
        if regression == "wallet":
            # A separate invocation creates a fresh cluster/database/nonce.
            # No Session/key regression has consumed this fixture or its IDs.
            tests = CONTROL / "tests/current-wallet-reconciliation"
            testenv = {**env, "PYTHONPATH": os.pathsep.join(map(str, (tests, keys, source / "scripts")))}
            run([sys.executable, "-B", tests / "test_migrate_current_wallet_reconciliation.py"],
                work, "wallet-pure", env=testenv, timeout=120)
            run([sys.executable, "-B", tests / "test_migrate_current_wallet_reconciliation_pg.py",
                "--fixture", work / "fixture.private.json", "--output", work / "regression"],
                work, "wallet-pg-regression", env=testenv, timeout=600)
            verified = json.loads((work / "regression/wallet-result.public.json").read_text())
            need(verified["status"] == "SYNTHETIC_PG_MECHANICS_VERIFIED"
                 and verified["production_approval_created"] is False
                 and verified["full_financial_import_complete"] is False,
                 "wallet mechanics did not pass or exceeded their evidence scope")
            result.update(status="PASS", wallet_reconciliation_mechanics=verified,
                          production_approval_created=False, full_financial_import_complete=False)
        elif regression == "identity-diagnostic":
            tests = CONTROL / "tests/current-identity"
            testenv = {**env, "PYTHONPATH": os.pathsep.join(map(str, (tests, keys, source / "scripts")))}
            run([sys.executable, "-B", tests / "diagnose_current_identity_pg.py",
                "--fixture", work / "fixture.private.json", "--output", work / "regression"],
                work, "identity-rollback-diagnostic", env=testenv, timeout=180)
            verified = json.loads((work / "regression/diagnostic-result.public.json").read_text())
            need(verified["status"] == "DIAGNOSTIC_COMPLETE_MIGRATION_STILL_REJECTED"
                 and verified["migration_passed"] is False
                 and verified["production_access"] is False
                 and verified["writer_connection_still_active"] is False
                 and verified["all_application_tables_unchanged"] is True
                 and verified["current_identity_receipt_absent"] is True,
                 "rollback-only diagnostic scope or postconditions differ")
            result.update(status="DIAGNOSTIC_COMPLETE_MIGRATION_STILL_REJECTED",
                          diagnostic=verified, migration_passed=False,
                          production_approval_created=False)
        elif regression == "quota-redemptions":
            tests = CONTROL / "tests/current-quota-redemptions"
            expected = {'migrate_current_quota_redemptions.py': '7442a32afac38c34b4431daeec8d271646a5c83313b2a49f33b89052ad5eea2d', 'quota_runtime.py': '171be10cfac059f1681548da3fcb8b097021adb6651b38c8b12e6f86e9697a22', 'test_current_quota_redemptions.py': 'd7af62105b28d68957700650f2453e035e147ca096d61160b3301f89d2cad993', 'test_current_quota_redemptions_pg.py': 'f4f03abd24b0e7a35c047e7b2e8909ccd7c7dea8ccc0984dc2b961061d65c784'}
            for filename, expected_sha in expected.items():
                need(sha(tests / filename) == expected_sha, "frozen quota input differs: " + filename)
            testenv = {**env, "PYTHONPATH": os.pathsep.join(map(str, (tests,
                CONTROL / "tests/current-key-production", CONTROL / "tests/current-identity-next",
                CONTROL / "tests/current-wallet-reconciliation", keys, source / "scripts")))}
            run([sys.executable, "-B", tests / "test_current_quota_redemptions.py"],
                work, "quota-pure", env=testenv, timeout=120)
            driver = tests / "test_current_quota_redemptions_pg.py"
            run([sys.executable, "-B", driver, "mechanics", "--fixture", work / "fixture.private.json",
                 "--output", work / "regression"], work, "quota-pg-regression", env=testenv, timeout=600)
            mechanics = json.loads((work / "regression/result.public.json").read_text())
            need(mechanics["status"] == "SYNTHETIC_PG_APPEND_VERIFIED"
                 and mechanics["production_approval_created"] is False
                 and mechanics["production_writes"] == 0, "quota append mechanics scope differs")
            runtime_driver = module("current_quota_http", tests / "quota_runtime.py")
            verified = runtime_driver.run_probe(sys.modules[__name__], output, work, sock, db, port,
                                                runtime, env, sql, driver, testenv)
            result.update(status="PASS", current_quota_append_mechanics=mechanics,
                          current_quota_redemption_runtime=verified,
                          production_approval_created=False, production_closure_tested=False)
        elif regression == "key-seam":
            tests = CONTROL / "tests/current-key-production"
            expected = {'current_key_production.py': 'b1e157a0db8b22ccc7a41263c54c0c6b6f0c4c2ba49fe825b91fc0e90a0ba8ba', 'current_key_lineage.py': '0835d57cb70a58ed8bdf368d5c25d588aca0139dfdb44389e2550981953c9da7', 'test_current_key_production.py': '58177f6aa0c1dbd9eaa7cc9c06c0d6a901c3cefabd569e914f807ea4a58d7213', 'test_current_key_production_pg.py': '47487389867938867c525cc1b399565abc656c7b229da149b6fcadb86ec46ccc', 'test_current_key_seam_fixture.py': 'dafb1cccd320dcc5070c16399b7affaeea1a4b2364174108861b1d19e7f3409e', 'current_key_seam_http.py': 'cde79b4513b040813bb9b9cb9ea7c00ba83df6f3be91464644bee9ce7cf60fa9', 'seed.sql': 'c7e52f6b908fb9e0603f3d764cd40a0aee6857938614030f287dbe88cd331fb1'}
            for filename, expected_sha in expected.items():
                need(sha(tests / filename) == expected_sha, "current key seam frozen input differs: " + filename)
            testenv = {**env, "PYTHONPATH": os.pathsep.join(map(str, (tests,
                CONTROL / "tests/current-identity-next", CONTROL / "tests/current-wallet-reconciliation",
                keys, source / "scripts")))}
            run([sys.executable, "-B", tests / "test_current_key_production.py"],
                work, "key-seam-pure", env=testenv, timeout=120)
            run([sys.executable, "-B", tests / "test_current_key_seam_fixture.py",
                "--fixture", work / "fixture.private.json", "--output", work / "regression"],
                work, "key-seam-pg", env=testenv, timeout=600)
            mechanics = json.loads((work / "regression/result.public.json").read_text())
            need(mechanics["status"] == "MECHANICS_VERIFIED_RUNTIME_PENDING"
                 and mechanics["production_closure_tested"] is False
                 and mechanics["production_authorized"] is False
                 and mechanics["production_access"] is False, "key seam mechanics scope differs")
            driver = module("current_key_seam_http", tests / "current_key_seam_http.py")
            verified = driver.run_probe(sys.modules[__name__], output, work, sock, db, port, runtime, env, sql)
            need(verified["status"] == "PASS" and verified["unrelated_application_state_unchanged"] is True
                 and verified["expected_reveal_audit_appended"] is True
                 and verified["production_closure_tested"] is False, "key seam HTTP scope differs")
            result.update(status="PASS", current_key_mechanics=mechanics, current_key_runtime=verified,
                          production_approval_created=False, production_closure_tested=False)
        elif regression == "identity-password":
            tests = CONTROL / "tests/current-identity-next"
            password_tests = CONTROL / "tests/current-identity-password"
            driver = password_tests / "test_current_identity_password_runtime.py"
            need(sha(driver) == "2787a8c11337348e38a70fb7a30b65de3732b40183920277dd53b0d1e9aaa1a8",
                 "frozen password runtime driver differs")
            testenv = {**env, "PYTHONPATH": os.pathsep.join(map(str, (password_tests, tests, keys, source / "scripts")))}
            run([sys.executable, "-B", driver, "prepare", "--fixture", work / "fixture.private.json",
                 "--output", work / "regression"], work, "identity-password-prepare", env=testenv, timeout=180)
            verified = password_runtime_probe(output, work, sock, db, port, runtime, env, sql, driver, testenv)
            result.update(status="PASS", password_runtime_validation=verified,
                          production_approval_created=False, provider_runtime_login="NOT_RUN")
        elif regression == "identity":
            tests = CONTROL / "tests/current-identity-next"
            testenv = {**env, "PYTHONPATH": os.pathsep.join(map(str, (tests, keys, source / "scripts")))}
            run([sys.executable, "-B", tests / "test_current_identity.py"],
                work, "identity-pure", env=testenv, timeout=120)
            run([sys.executable, "-B", tests / "test_current_identity_pg.py",
                "--fixture", work / "fixture.private.json", "--output", work / "regression"],
                work, "identity-pg-regression", env=testenv, timeout=600)
            verified = json.loads((work / "regression/result.public.json").read_text())
            need(verified["status"] == "MECHANICS_VERIFIED_RUNTIME_PENDING"
                 and verified["production_authorized"] is False
                 and verified["production_access"] is False
                 and verified["real_google_login"] == "NOT_RUN"
                 and verified["password_runtime_login"] == "NOT_RUN",
                 "identity mechanics did not pass or exceeded their evidence scope")
            result.update(status="PASS", identity_completion_mechanics=verified,
                          production_approval_created=False, provider_runtime_login="NOT_RUN")
        else:
            testenv = {**env, "PYTHONPATH": str(keys) + os.pathsep + str(source / "scripts")}
            run([sys.executable, "-B", keys / "test_migrate_current_runtime_keys_pg.py",
                "--fixture", work / "fixture.private.json", "--output", work / "regression"],
                work, "pg-regression", env=testenv, timeout=600)
            verified = json.loads((work / "regression/result.public.json").read_text())
            need(verified["status"] == "MECHANICS_VERIFIED_RUNTIME_PENDING", "regression did not pass")
            result.update(status="MECHANICS_VERIFIED_RUNTIME_PENDING", mechanics=verified,
                          runtime_authentication_and_quota="NOT_RUN")
            financial_readback = """SELECT jsonb_build_object(
 'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),
 'journals',(SELECT count(*) FROM core_billing.ledger_journals),
 'opening_consumption',(SELECT jsonb_agg(to_jsonb(b) ORDER BY key_id) FROM core_charging.budget_opening_consumption b));"""
            before_http = sql(financial_readback)
            events = runtime_probe(source, output, work, sock, db, port, runtime, env, sql)
            need(sql(financial_readback) == before_http, "auth/recovery/projection changed money or historical quota")
            result.pop("runtime_authentication_and_quota", None)
            result.update(status="PASS", runtime_authentication_and_quota_projection="PASS",
                          runtime_events=events, auth_recovery_projection_financial_writes=False)
    except BaseException as error:
        result["error"] = type(error).__name__ + ": " + str(error)
        raise
    finally:
        if pg is not None and pg.poll() is None:
            stop = subprocess.run(["pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop"],
                                  capture_output=True, env=env, timeout=30)
            (work / "stop.stdout.log").write_bytes(stop.stdout)
            (work / "stop.stderr.log").write_bytes(stop.stderr)
            result["postgres_stop_exit"] = stop.returncode
            result["postgres_exit"] = pg.wait(timeout=10)
        result["socket_stopped"] = not (sock / (".s.PGSQL." + port)).exists()
        result["finished_utc"] = datetime.now(timezone.utc).isoformat()
        save(output / "result.public.json", result)
        source_check(source)
    need(result["socket_stopped"] and result.get("postgres_stop_exit") == 0,
         "isolated PostgreSQL did not stop cleanly")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("acquire", "run"))
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--regression", choices=("keys", "wallet", "identity", "identity-diagnostic", "identity-password", "key-seam", "quota-redemptions"), default="keys")
    args = parser.parse_args()
    if args.phase == "acquire":
        acquire(args.source.resolve(), args.output.resolve())
    else:
        execute(args.source.resolve(), args.output.resolve(), regression=args.regression)
