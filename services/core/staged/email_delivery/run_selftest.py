#!/usr/bin/env python3
"""Compile staged mail tests from existing locked rlibs and an isolated PG18."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--rustc", type=Path, required=True)
parser.add_argument("--deps", type=Path, required=True, help="existing target/debug/deps directory")
parser.add_argument("--evidence", type=Path, required=True, help="new or empty evidence directory")
args = parser.parse_args()

module = Path(__file__).resolve().parent
core = module.parents[1]
deps = args.deps.resolve(strict=True)
evidence = args.evidence.resolve()
if evidence.exists() and any(evidence.iterdir()):
    raise SystemExit("evidence directory must be new or empty")
evidence.mkdir(parents=True, mode=0o700, exist_ok=True)
scratch = Path(tempfile.mkdtemp(prefix="lmm-email-pg-", dir="/tmp"))
os.chmod(scratch, 0o700)
env = {key: value for key, value in os.environ.items()
       if not key.startswith(("PG", "EMAIL_TEST_"))}
commands = []
started = False
status = 1

def run(command, log, **kwargs):
    start = time.monotonic()
    with (evidence / log).open("wb") as output:
        result = subprocess.run([str(item) for item in command], env=env,
                                stdout=output, stderr=subprocess.STDOUT, **kwargs)
    commands.append({"command": [str(item) for item in command],
                     "exit_code": result.returncode,
                     "elapsed_seconds": round(time.monotonic() - start, 3),
                     "log": log})
    return result.returncode

def one(crate, prefix=None):
    pattern = f"lib{crate.replace('-', '_')}-*.rlib"
    choices = sorted(deps.glob(pattern))
    if prefix:
        choices = [path for path in choices if path.name.startswith(prefix)]
    if len(choices) != 1:
        raise RuntimeError(f"expected one cached {crate} rlib")
    return choices[0]

try:
    selected = {
        "hmac": one("hmac", "libhmac-548dfe3a060ff810"),
        "sha2": one("sha2", "libsha2-f4bbab4e3ff92b9d"),
        "rand": one("rand", "librand-6f5c2849f7fb2219"),
        "sqlx": one("sqlx", "libsqlx-b0d2ff7a8a592cbe"),
        "tokio": one("tokio", "libtokio-4be1fe929df05a1a"),
        "zeroize": one("zeroize", "libzeroize-d74d6a75977cdced"),
        "subtle": one("subtle", "libsubtle-cb0346f0159cd98d"),
        "aes_gcm": one("aes-gcm", "libaes_gcm-b9b978e0de7d34aa"),
        "lettre": one("lettre", "liblettre-2182c1b39c83f5c0"),
        "reqwest": one("reqwest", "libreqwest-d83504b525fac7e8"),
    }
    inputs = [module / name for name in
              ("mod.rs", "crypto.rs", "smtp.rs", "store.rs", "schema.sql",
               "test_grants.sql", "README.md", "run_selftest.py")]
    inputs += [module / "store/tests.rs", module / "store/reset_smtp_cases.rs", module / "reset_link.rs"]
    inputs += [core / "src/identity/email" / name for name in
               ("mod.rs", "address.rs", "store.rs", "schema.sql",
                "test_grants.sql", "tests.rs")]
    inputs += [core / "schema/identity.sql", core / "Cargo.lock"]
    hashes = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}
    (evidence / "inputs.json").write_text(json.dumps(hashes, indent=2) + "\n")
    (evidence / "dependencies.json").write_text(json.dumps({
        name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        for name, path in selected.items()
    }, indent=2) + "\n")
    if run([args.rustc, "--version", "--verbose"], "rustc-version.log"):
        raise RuntimeError("rustc version query failed")
    if run(["postgres", "--version"], "postgres-version.log"):
        raise RuntimeError("PostgreSQL version query failed")

    harness = scratch / "harness.rs"
    harness.write_text(
        f'#[path = {json.dumps(str(core / "src/identity/email/mod.rs"))}]\n'
        'pub mod email_challenges;\n'
        'pub mod identity { pub use crate::email_challenges as email; }\n'
        f'#[path = {json.dumps(str(module / "mod.rs"))}]\n'
        'pub mod email_delivery;\n'
    )
    binary = scratch / "email-delivery-tests"
    command = [args.rustc, "--crate-name", "email_delivery_slice", "--edition=2024",
               "--test", harness, "-C", "debuginfo=0", "-C", "strip=symbols",
               "-C", "linker=clang", "-C", "link-arg=-fuse-ld=mold", "-o", binary]
    command += ["-L", f"dependency={deps}"]
    for native in deps.parent.glob("build/*/out"):
        command += ["-L", f"native={native}"]
    for name, path in selected.items():
        command += ["--extern", f"{name}={path}"]
    status = run(command, "compile.log")
    if status:
        raise RuntimeError("staged leaf test compilation failed")

    if run(["initdb", "-D", scratch / "data", "-U", "email_owner", "--no-locale",
            "--encoding=UTF8", "--auth-local=trust", "--auth-host=reject",
            "--wal-segsize=1", "--no-sync"], "initdb.log"):
        raise RuntimeError("isolated initdb failed")
    socket = scratch / "s"
    socket.mkdir(mode=0o700)
    pg_options = (f"-c listen_addresses='' -c unix_socket_directories={socket} -p 55841 "
                  "-c shared_buffers=8MB -c max_connections=16 "
                  "-c min_wal_size=2MB -c max_wal_size=4MB")
    if run(["pg_ctl", "-D", scratch / "data", "-l", scratch / "postgres.log",
            "-o", pg_options, "-w", "start"], "start.log"):
        raise RuntimeError("isolated PostgreSQL start failed")
    started = True
    psql = ["psql", "-X", "-h", socket, "-p", "55841", "-U", "email_owner",
            "-d", "postgres", "-v", "ON_ERROR_STOP=1"]
    setup = ("CREATE ROLE email_runtime LOGIN NOSUPERUSER NOCREATEDB "
             "NOCREATEROLE NOINHERIT NOBYPASSRLS;\nCREATE DATABASE email_slice;\n")
    if run(psql, "provision.log", input=setup.encode()):
        raise RuntimeError("isolated database provisioning failed")
    env["EMAIL_TEST_OWNER_URL"] = (
        f"postgresql://email_owner@localhost:55841/email_slice?host={socket}&sslmode=disable")
    env["EMAIL_TEST_RUNTIME_URL"] = (
        f"postgresql://email_runtime@localhost:55841/email_slice?host={socket}&sslmode=disable")
    status = run([binary, "--include-ignored", "--test-threads=1", "--nocapture"],
                 "tests.log")
finally:
    sizes = {}
    if scratch.exists():
        sizes = {str(path.relative_to(scratch)): path.stat().st_size
                 for path in scratch.rglob("*") if path.is_file()}
    if started:
        stopped = run(["pg_ctl", "-D", scratch / "data", "-m", "fast", "-w", "stop"],
                      "stop.log") == 0
        if (scratch / "postgres.log").exists():
            shutil.copyfile(scratch / "postgres.log", evidence / "postgres.log")
    else:
        stopped = True
    peak_bytes = sum(sizes.values())
    if stopped:
        shutil.rmtree(scratch)
    (evidence / "temporary-sizes.json").write_text(json.dumps({
        "fixture_bytes_before_cleanup": peak_bytes,
        "fixture_removed": not scratch.exists(),
    }, indent=2) + "\n")
    (evidence / "results.json").write_text(json.dumps({
        "exit_code": status if stopped else 1,
        "commands": commands,
        "fixture_removed": not scratch.exists(),
        "scope": "locked leaf dependencies; existing challenge tests; isolated PG18; local SMTP sink",
    }, indent=2) + "\n")
    if not stopped:
        raise RuntimeError("isolated PostgreSQL did not stop; fixture retained for recovery")
raise SystemExit(status)
