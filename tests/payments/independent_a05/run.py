import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import threading
import time

import argparse
parser = argparse.ArgumentParser(description="Independent A05 real PG/Core/module-worker acceptance; local synthetic data only")
parser.add_argument("--source-root", type=Path, required=True)
parser.add_argument("--core-binary", type=Path, required=True)
parser.add_argument("--test-binary", type=Path, required=True)
parser.add_argument("--core-sha256", required=True)
parser.add_argument("--test-sha256", required=True)
parser.add_argument("--output-root", type=Path, required=True)
parser.add_argument("--compiled-inputs", type=Path, required=True)
parser.add_argument("--case", choices=("automatic", "binding", "over-refund"), default="automatic")
args = parser.parse_args()
ROOT=args.output_root.resolve()
SOURCE=args.source_root.resolve()
CORE=args.core_binary.resolve()
TEST=args.test_binary.resolve()
assert hashlib.sha256(CORE.read_bytes()).hexdigest() == args.core_sha256, "Core binary digest mismatch"
assert hashlib.sha256(TEST.read_bytes()).hexdigest() == args.test_sha256, "worker binary digest mismatch"
assert not ROOT.exists(), "output root must be fresh"
assert len(str(ROOT/"core.sock")) <= 100, "choose a short absolute output path"
ROOT.mkdir(mode=0o700)
(ROOT/"tmp").mkdir(mode=0o700)
manifest=json.loads(args.compiled_inputs.read_text())
for name,digest in manifest.items():
    assert hashlib.sha256((SOURCE/name).read_bytes()).hexdigest()==digest, name
(ROOT/"binary-binding.json").write_text(json.dumps({"manifest_sha256":hashlib.sha256(args.compiled_inputs.read_bytes()).hexdigest(),"core_sha256":hashlib.sha256(CORE.read_bytes()).hexdigest(),"test_sha256":hashlib.sha256(TEST.read_bytes()).hexdigest()}))
os.umask(0o077)
ENV = {"PATH": "/usr/bin:/bin", "HOME": os.environ["HOME"], "TMPDIR": str(ROOT/"tmp"),
       "GOTMPDIR": str(ROOT/"tmp"), "GOTOOLCHAIN": "local", "GOPROXY": "off",
       "GOSUMDB": "off", "GOMAXPROCS": "1", "GOMEMLIMIT": "128MiB",
       "GOCACHE": os.environ.get("GOCACHE", str(Path.home()/".cache/go-build")),
       "GOPATH": str(Path.home()/"go")}
commands = []
stop_watch = threading.Event()
violation = threading.Event()


def observation():
    memory = int(next(x.split()[1] for x in Path("/proc/meminfo").read_text().splitlines()
                      if x.startswith("MemAvailable:")))
    rust = []
    for p in Path("/proc").iterdir():
        if p.name.isdigit():
            try:
                if (p/"comm").read_text().strip() in ("cargo", "rustc"):
                    rust.append(int(p.name))
            except OSError:
                pass
    row = {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
           "mem_available_kib": memory, "other_cargo_rustc": rust}
    with (ROOT/"resources.jsonl").open("a") as f:
        f.write(json.dumps(row)+"\n")
    return row


def gate():
    o = observation()
    if o["mem_available_kib"] < 2*1024*1024 or o["other_cargo_rustc"]:
        raise RuntimeError("resource gate closed")


def watch():
    while not stop_watch.wait(1):
        o = observation()
        if o["mem_available_kib"] < 2*1024*1024 or o["other_cargo_rustc"]:
            violation.set()


def run(name, argv, *, cwd=ROOT, data=None, env=None, deadline=600):
    gate()
    if violation.is_set():
        raise RuntimeError("resource violation already recorded")
    started = time.monotonic()
    log = ROOT/(name+".log")
    print("START", name, flush=True)
    with log.open("wb") as f:
        p = subprocess.Popen([str(x) for x in argv], cwd=cwd, env=env or ENV,
                             stdin=subprocess.PIPE if data is not None else subprocess.DEVNULL,
                             stdout=f, stderr=subprocess.STDOUT, start_new_session=True)
        if data is not None:
            p.stdin.write(data)
            p.stdin.close()
        forced = None
        while p.poll() is None:
            if violation.is_set() or time.monotonic()-started > deadline:
                forced = "resource_violation" if violation.is_set() else "deadline"
                os.killpg(p.pid, signal.SIGKILL)
                break
            time.sleep(0.25)
        code = p.wait()
    row = {"stage": name, "argv": [str(x) for x in argv], "cwd": str(cwd),
           "exit_code": code, "seconds": round(time.monotonic()-started, 3),
           "log": str(log), "forced_stop": forced}
    commands.append(row)
    (ROOT/"commands.json").write_text(json.dumps(commands, indent=2)+"\n")
    print("END", name, code, row["seconds"], flush=True)
    if code != 0:
        raise RuntimeError(name+" failed; raw log retained")
    return log


def private(name, text):
    path = ROOT/name
    path.write_text(text)
    path.chmod(0o600)
    return path


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    lock = open("/tmp/lmm-delegate/heavy-build.lock", "a")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    gate()
    watcher = threading.Thread(target=watch, daemon=True)
    watcher.start()
    pg = ROOT/"pg"
    pg.mkdir(mode=0o700)
    pg_started = False
    try:
        run("initdb", ["initdb", "-D", pg/"data", "-U", "a05_owner",
            "--auth-local=trust", "--auth-host=trust", "--encoding=UTF8",
            "--no-locale", "--wal-segsize=1", "--no-sync"])
        port = free_port()
        run("pg-start", ["pg_ctl", "-D", pg/"data", "-l", pg/"postgres.log",
            "-o", f"-h 127.0.0.1 -p {port} -k {pg} -c shared_buffers=16MB -c max_connections=35",
            "-w", "start"])
        pg_started = True
        def sql(name, database, text):
            return run(name, ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1",
                       "-h", "127.0.0.1", "-p", str(port), "-U", "a05_owner",
                       "-d", database], data=text.encode())
        sql("create-databases", "postgres", """
CREATE ROLE lmm_core_owner NOLOGIN;
CREATE ROLE a05_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE a05_go LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE DATABASE a05_core_test TEMPLATE template0;
CREATE DATABASE a05_go_test TEMPLATE template0;
REVOKE CONNECT,TEMPORARY,CREATE ON DATABASE a05_core_test,a05_go_test FROM PUBLIC;
GRANT CONNECT ON DATABASE a05_core_test TO a05_runtime;
GRANT CONNECT ON DATABASE a05_go_test TO a05_go;
""")
        owner_url = f"postgres://a05_owner@127.0.0.1:{port}/a05_core_test?sslmode=disable"
        go_owner_url = f"postgres://a05_owner@127.0.0.1:{port}/a05_go_test?sslmode=disable"
        private("owner.url", owner_url)
        private("runtime.url", f"postgres://a05_runtime@127.0.0.1:{port}/a05_core_test?sslmode=disable")
        private("go.url", f"postgres://a05_go@127.0.0.1:{port}/a05_go_test?sslmode=disable")
        identity = "a05_independent_identity_0123456789abcdef0123456789"
        token = "a05_independent_payments_0123456789abcdef0123456789"
        secret = "whsec_a05_independent_synthetic_fixture"
        private("identity.token", identity)
        private("payment.token", token)
        private("webhook.secret", secret)
        permission = private("payments.toml", f"""version=1
provider='stripe-test'
[permissions]
prepare=true
credit_evidence=true
read_user=true
reconcile=true
proof_ingress=true
proof_read=true
refund_user=true
refund_evidence=true
[[channels]]
id=1
merchant='acct_independent_a05'
webhook_secret_file='{ROOT}/webhook.secret'
max_usd_cents=100000
""")
        http_port = free_port()
        def config(url):
            return f"""version=1
[server]
listen='127.0.0.1:{http_port}'
[database]
url_file='{url}'
[rpc]
socket='{ROOT}/core.sock'
token_file='{ROOT}/identity.token'
[payments]
config_file='{permission}'
token_file='{ROOT}/payment.token'
"""
        private("owner.toml", config(ROOT/"owner.url"))
        private("runtime.toml", config(ROOT/"runtime.url"))
        run("core-install", [CORE, "--config", ROOT/"owner.toml", "init-db"])
        run("core-runtime-grants", ["psql", "-X", "-q", "-v", "ON_ERROR_STOP=1",
            "-h", "127.0.0.1", "-p", str(port), "-U", "a05_owner", "-d", "a05_core_test",
            "-v", "runtime_role=a05_runtime", "-v", "enable_payments=true",
            "-v", "enable_events=true", "-f", SOURCE/"deployment/postgres/grant-core-runtime.sql"])
        sql("go-schema", "a05_go_test", (SOURCE/"services/extensions/internal/modules/payments/schema.sql").read_text()+"""
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO a05_go;
GRANT SELECT ON payment_storage_guard TO a05_go;
GRANT SELECT,INSERT,UPDATE ON payment_orders,payment_events TO a05_go;
""")
        private("test-config.json", json.dumps({
            "root": str(ROOT), "core_binary": str(CORE),
            "core_sha256": hashlib.sha256(CORE.read_bytes()).hexdigest(),
            "owner_config": str(ROOT/"owner.toml"), "core_config": str(ROOT/"runtime.toml"),
            "core_url": owner_url, "go_owner_url": go_owner_url,
            "go_url": (ROOT/"go.url").read_text(), "go_database": "a05_go_test",
            "socket": str(ROOT/"core.sock"), "token": token, "secret": secret,
            "endpoint": f"http://127.0.0.1:{http_port}"}))
        run("independent-acceptance", [TEST, "-test.run", {"automatic":"^TestIndependentA05Automatic$", "binding":"^TestIndependentA05$/^binding_capacity_cross_entry_concurrency$", "over-refund":"^TestIndependentA05Automatic$/^both_successes_over_payment_remain_fenced$"}[args.case],
            "-test.v", "-test.timeout=12m"], env=ENV|{"A05_CONFIG": str(ROOT/"test-config.json")},
            deadline=720)
        private("execution-result.json", json.dumps({"result": "PASS",
            "scope": "independent real PostgreSQL and Core RPC / Go module OS processes; synthetic provider",
            "resource_violation": violation.is_set()}, indent=2)+"\n")
    finally:
        stop_watch.set()
        watcher.join(timeout=3)
        if pg_started:
            # Shutdown is allowed even if the resource launch gate is closed.
            with (ROOT/"pg-stop.log").open("wb") as f:
                stopped = subprocess.run(["pg_ctl", "-D", str(pg/"data"), "-m", "immediate", "-w", "stop"],
                    env=ENV, stdout=f, stderr=subprocess.STDOUT, timeout=15)
            private("cleanup-state.json", json.dumps({"postgres_stop_exit": stopped.returncode,
                "data_retained_pending_review": True})+"\n")
        fcntl.flock(lock, fcntl.LOCK_UN)
        lock.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        private("execution-failure.json", json.dumps({"result": "FAIL_OR_BLOCKED",
            "type": type(e).__name__, "reason": str(e)})+"\n")
        raise
