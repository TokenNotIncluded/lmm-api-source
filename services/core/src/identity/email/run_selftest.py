#!/usr/bin/env python3
"""Compile this exact leaf module using existing rlibs; test on disposable PG18."""
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
parser.add_argument("--extern", action="append", required=True, help="name=/absolute/path/to/cached.rlib")
parser.add_argument("--evidence", type=Path, required=True)
args = parser.parse_args()
module = Path(__file__).resolve().parent
core = module.parents[2]
evidence = args.evidence.resolve()
evidence.mkdir(parents=True, mode=0o700, exist_ok=True)
scratch = Path(tempfile.mkdtemp(prefix="lmm-email-pg-", dir="/tmp"))
os.chmod(scratch, 0o700)
env = {k: v for k, v in os.environ.items() if not k.startswith(("PG", "EMAIL_TEST_"))}
records = []
started = False
status = 1
paths = sorted(module.glob("*.rs")) + [module / "schema.sql", module / "test_grants.sql",
                                      core / "schema/identity.sql", core / "Cargo.lock"]
inputs = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
(evidence / "inputs.json").write_text(json.dumps(inputs, indent=2) + "\n")


def run(command, log, **kwargs):
    start = time.monotonic()
    with (evidence / log).open("wb") as output:
        result = subprocess.run([str(c) for c in command], env=env,
                                stdout=output, stderr=subprocess.STDOUT, **kwargs)
    records.append({"command": [str(c) for c in command], "exit_code": result.returncode,
                    "elapsed_seconds": round(time.monotonic() - start, 3), "log": log})
    return result.returncode


try:
    externs = {}
    for value in args.extern:
        name, path = value.split("=", 1)
        path = Path(path).resolve(strict=True)
        if path.suffix != ".rlib" or name in externs:
            raise ValueError("expected one existing rlib per dependency")
        externs[name] = path
    if set(externs) != {"hmac", "sha2", "rand", "sqlx", "tokio", "zeroize", "subtle"}:
        raise ValueError("supply exactly the seven module dependencies")
    dependency_dirs = {path.parent for path in externs.values()}
    (evidence / "dependencies.json").write_text(json.dumps({
        name: {"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
        for name, p in externs.items()
    }, indent=2) + "\n")
    run([args.rustc, "--version", "--verbose"], "rustc-version.log")
    run(["postgres", "--version"], "postgres-version.log")
    harness = scratch / "harness.rs"
    harness.write_text(f'#[path = {json.dumps(str(module / "mod.rs"))}]\npub mod email;\n')
    binary = scratch / "email-tests"
    command = [args.rustc, "--crate-name", "email_slice", "--edition=2024",
               "--test", harness, "-C", "debuginfo=0", "-C", "strip=symbols",
               "-C", "linker=clang", "-C", "link-arg=-fuse-ld=mold", "-o", binary]
    for directory in dependency_dirs:
        command += ["-L", f"dependency={directory}"]
        for native in directory.parent.glob("build/*/out"):
            command += ["-L", f"native={native}"]
    for name, path in externs.items():
        command += ["--extern", f"{name}={path}"]
    status = run(command, "compile.log")
    if status:
        raise RuntimeError("leaf compilation failed")
    status = 1
    # 1 MB WAL segments keep this one-purpose fixture and binary below 100 MB.
    if run(["initdb", "-D", scratch / "data", "-U", "email_owner", "--no-locale",
            "--encoding=UTF8", "--auth-local=trust", "--auth-host=reject",
            "--wal-segsize=1", "--no-sync"], "initdb.log"):
        raise RuntimeError("isolated initdb failed")
    socket = scratch / "s"
    socket.mkdir(mode=0o700)
    if run(["pg_ctl", "-D", scratch / "data", "-l", scratch / "postgres.log", "-o",
            f"-c listen_addresses='' -c unix_socket_directories={socket} -p 55583 "
            "-c shared_buffers=8MB -c max_connections=16 "
            "-c min_wal_size=2MB -c max_wal_size=4MB", "-w", "start"], "start.log"):
        raise RuntimeError("isolated PostgreSQL start failed")
    started = True
    if run(["psql", "-X", "-h", socket, "-p", "55583", "-U", "email_owner",
            "-d", "postgres", "-v", "ON_ERROR_STOP=1"], "provision.log",
           input=b"CREATE ROLE email_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;\nCREATE DATABASE email_slice;\n"):
        raise RuntimeError("isolated role/database creation failed")
    env["EMAIL_TEST_OWNER_URL"] = f"postgresql://email_owner@localhost:55583/email_slice?host={socket}&sslmode=disable"
    env["EMAIL_TEST_RUNTIME_URL"] = f"postgresql://email_runtime@localhost:55583/email_slice?host={socket}&sslmode=disable"
    status = run([binary, "--include-ignored", "--test-threads=1", "--nocapture"], "tests.log")
finally:
    peak_bytes = sum(p.stat().st_size for p in scratch.rglob("*") if p.is_file())
    changed = [str(p) for p in paths if hashlib.sha256(p.read_bytes()).hexdigest() != inputs[str(p)]]
    if started:
        stopped = run(["pg_ctl", "-D", scratch / "data", "-m", "fast", "-w", "stop"], "stop.log") == 0
        shutil.copyfile(scratch / "postgres.log", evidence / "postgres.log")
    else:
        stopped = True
    if stopped:
        shutil.rmtree(scratch)
    if changed:
        status = 1
    (evidence / "results.json").write_text(json.dumps({
        "exit_code": status, "commands": records, "changed_inputs": changed,
        "fixture_bytes_before_cleanup": peak_bytes,
        "fixture_removed": not scratch.exists(),
        "scope": "exact email leaf, real isolated PG18, restricted runtime; not full Core/HTTP/Web",
    }, indent=2) + "\n")
raise SystemExit(status)
