#!/usr/bin/env python3
"""Self-test email business with a private source copy and read-only Rust cache."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--cache-target", type=Path, required=True)
parser.add_argument("--rust-bin", type=Path, required=True)
parser.add_argument("--evidence", type=Path, required=True)
parser.add_argument("--expected-source", action="append", required=True,
                    help="freeze acknowledgement: repository-relative-path=sha256")
parser.add_argument("--test-filter", help="run only this exact failed test on a corrected retry")
args = parser.parse_args()
core = Path(__file__).resolve().parents[1]
repo = core.parents[1]
evidence = args.evidence.resolve()
evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
scratch = Path(tempfile.mkdtemp(prefix="lmm-email-business-", dir="/tmp"))
os.chmod(scratch, 0o700)
env = {k: v for k, v in os.environ.items()
       if not k.startswith(("PG", "EMAIL_BUSINESS_", "DATABASE_URL", "RUSTFLAGS"))}
records = []
started = False
status = 1


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, log, **kwargs):
    start = time.monotonic()
    with (evidence / log).open("wb") as output:
        result = subprocess.run([str(x) for x in command], cwd=core, env=env,
                                stdout=output, stderr=subprocess.STDOUT, **kwargs)
    records.append({"command": [str(x) for x in command], "cwd": str(core),
                    "exit_code": result.returncode, "log": str(evidence / log),
                    "elapsed_seconds": round(time.monotonic() - start, 3)})
    (evidence / "commands.json").write_text(json.dumps(records, indent=2) + "\n")
    if result.returncode:
        raise RuntimeError(f"{log} failed; see evidence")


try:
    acknowledged = {}
    for value in args.expected_source:
        relative, digest = value.rsplit("=", 1)
        if len(digest) != 64 or not (repo / relative).resolve().is_relative_to(repo):
            raise ValueError("expected a repository source path and SHA256")
        if sha(repo / relative) != digest:
            raise ValueError(f"acknowledged source changed before snapshot: {relative}")
        acknowledged[relative] = digest
    (evidence / "freeze-acknowledgement.json").write_text(
        json.dumps(acknowledged, indent=2) + "\n")
    frozen = scratch / "source"
    frozen_core = frozen / "services/core"
    for directory in ["src", "schema", "tests/core_schema"]:
        shutil.copytree(core / directory, frozen_core / directory)
    for relative in ["services/core/Cargo.toml", "services/core/Cargo.lock",
                     "services/core/tests/email_business/cases.rs",
                     "services/extensions/internal/modules/toolmarket/builtin_catalog.json",
                     "contracts/assistant-tools-v1.json",
                     "contracts/core/v1/funding-cases.json",
                     "deployment/postgres/grant-core-runtime.sql"]:
        target = frozen / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / relative, target)
    inputs = {str(p.relative_to(frozen)): sha(p) for p in frozen.rglob("*") if p.is_file()}
    changed = [p for p, digest in inputs.items() if sha(repo / p) != digest]
    if changed or any(inputs.get(p) != digest for p, digest in acknowledged.items()):
        raise RuntimeError(f"source changed during snapshot; compilation not started: {changed}")
    (evidence / "source-inputs.json").write_text(json.dumps(inputs, indent=2) + "\n")
    # This is the only source-composition patch; never applied to the workspace.
    entry = frozen_core / "src/identity/mod.rs"
    patch = "\npub mod email_business;\n"
    entry.write_text(entry.read_text() + patch)
    (evidence / "composition-patch.txt").write_text(patch)
    (evidence / "patched-inputs.json").write_text(json.dumps(
        {**inputs, str(entry.relative_to(frozen)): sha(entry)}, indent=2) + "\n")

    debug = args.cache_target.resolve() / "debug"
    deps = debug / "deps"
    fp = debug / ".fingerprint"
    candidates = list(deps.glob("liblmm_core-*.rlib"))
    if len(candidates) != 1:
        raise ValueError("select a cache with exactly one compiled Core library")
    suffix = candidates[0].stem.removeprefix("liblmm_core-")
    base = json.loads((fp / f"lmm-core-{suffix}/lib-lmm_core.json").read_text())
    externs = {}
    for _, name, _, fingerprint in base["deps"]:
        if name == "build_script_build":
            continue
        for directory in fp.iterdir():
            marker = directory / f"lib-{name}"
            if marker.is_file() and int.from_bytes(bytes.fromhex(marker.read_text()), "little") == fingerprint:
                artifact = deps / f"lib{name}-{directory.name.rsplit('-', 1)[1]}.rlib"
                if artifact.is_file():
                    externs[name] = artifact
                    break
        if name not in externs:
            raise ValueError(f"missing exact cached dependency: {name}")
    declared = tomllib.loads((frozen_core / "Cargo.toml").read_text())["dependencies"]
    if set(externs) != {n.replace("-", "_") for n in declared}:
        raise ValueError("cache direct dependencies differ from the source manifest")
    (evidence / "dependencies.json").write_text(json.dumps({
        n: {"path": str(p), "sha256": sha(p)} for n, p in externs.items()
    }, indent=2) + "\n")
    generated = sorted(debug.glob("build/lmm-core-*/out/lmm.core.v1.rs"))
    if not generated or len({sha(p) for p in generated}) != 1:
        raise ValueError("cached generated protocol outputs must agree")
    env.update({"OUT_DIR": str(generated[0].parent), "CARGO_MANIFEST_DIR": str(frozen_core),
                "CARGO_PKG_VERSION": "0.1.0"})
    (evidence / "protocol.json").write_text(json.dumps({
        "generated_path": str(generated[0]), "generated_sha256": sha(generated[0]),
        "proto_inputs": {str(p.relative_to(repo)): sha(p)
                         for p in (repo / "contracts/proto").rglob("*.proto")}
    }, indent=2) + "\n")
    rustc = args.rust_bin / "rustc"
    run([rustc, "--version", "--verbose"], "rustc-version.log")
    if not (evidence / "rustc-version.log").read_text().startswith("rustc 1.99.0 "):
        raise ValueError("the cache requires the verified Rust 1.99.0 toolchain")
    run(["postgres", "--version"], "postgres-version.log")
    run([args.rust_bin / "rustfmt", "--edition", "2024", "--check",
         core / "src/identity/email_business.rs", core / "tests/email_business/cases.rs"], "rustfmt.log")
    search = ["-L", f"dependency={deps}"]
    for native in debug.glob("build/*/out"):
        search += ["-L", f"native={native}"]
    link = ["-C", "debuginfo=0", "-C", "codegen-units=1",
            "-C", "linker=clang", "-C", "link-arg=-fuse-ld=mold"]
    external = []
    for name, path in externs.items():
        external += ["--extern", f"{name}={path}"]
    library = scratch / "liblmm_core.rlib"
    # Compile current Core once, against prebuilt dependencies. No Cargo/cache write.
    run(["nice", "-n", "10", rustc, "--crate-name", "lmm_core", "--edition=2024",
         "--crate-type", "rlib", frozen_core / "src/lib.rs",
         *search, *external, *link, "-o", library], "compile-core.log")
    binary = scratch / "email-business-tests"
    test_externs = ["--extern", f"lmm_core={library}"]
    for name in ["sqlx", "tokio", "sha2", "rand", "totp_rs"]:
        test_externs += ["--extern", f"{name}={externs[name]}"]
    test_source = frozen_core / "tests/email_business/cases.rs"
    run([rustc, "--crate-name", "email_business_tests", "--edition=2024", "--test",
         test_source, *search, *test_externs, *link, "-o", binary], "compile-tests.log")
    run([args.rust_bin / "clippy-driver", "--crate-name", "email_business_tests",
         "--edition=2024", "--test", test_source, *search, *test_externs,
         "--emit=metadata", "-o", scratch / "test-check.rmeta",
         "-W", "clippy::all", "-D", "warnings"], "clippy-tests.log")
    run(["initdb", "-D", scratch / "data", "-U", "email_business_installer",
         "--no-locale", "--encoding=UTF8", "--auth-local=trust", "--auth-host=reject",
         "--wal-segsize=1", "--no-sync"], "initdb.log")
    socket = scratch / "s"
    socket.mkdir(mode=0o700)
    run(["pg_ctl", "-D", scratch / "data", "-l", scratch / "postgres.log", "-o",
         f"-c listen_addresses='' -c unix_socket_directories={socket} -p 55587 "
         "-c shared_buffers=8MB -c max_connections=24 -c min_wal_size=2MB -c max_wal_size=4MB",
         "-w", "start"], "start.log")
    started = True
    env.update({"PGHOST": str(socket), "PGPORT": "55587", "PGUSER": "email_business_installer",
                "PGDATABASE": "postgres", "PGSSLMODE": "disable"})
    run(["psql", "-X", "-v", "ON_ERROR_STOP=1"], "roles.log",
        input=b"CREATE ROLE email_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;\n"
              b"CREATE ROLE lmm_core_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;\n")
    env["EMAIL_BUSINESS_FIXTURE_ROOT"] = str(scratch)
    env["EMAIL_BUSINESS_OWNER_URL"] = (
        f"postgresql://email_business_installer@localhost:55587/postgres?host={socket}&sslmode=disable")
    test_command = [binary, "--include-ignored", "--test-threads=1", "--nocapture"]
    if args.test_filter:
        test_command += ["--exact", args.test_filter]
    run(test_command, "tests.log")
    status = 0
finally:
    peak_bytes = sum(p.stat().st_size for p in scratch.rglob("*") if p.is_file())
    if started:
        run(["pg_ctl", "-D", scratch / "data", "-m", "fast", "-w", "stop"], "stop.log")
        shutil.copyfile(scratch / "postgres.log", evidence / "postgres.log")
    shutil.rmtree(scratch)
    (evidence / "results.json").write_text(json.dumps({
        "exit_code": status, "commands": records, "scratch_bytes_before_cleanup": peak_bytes,
        "fixture_removed": not scratch.exists(),
        "scope": "current frozen Core rlib + new email business tests; private PG; not full test suite",
        "test_filter": args.test_filter,
        "mail": "no sender; test-only non-secret notification marker is not encrypted outbox acceptance",
        "cache": "read-only reuse; no longer needed by this run",
    }, indent=2) + "\n")
raise SystemExit(status)
