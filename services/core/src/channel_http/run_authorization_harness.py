#!/usr/bin/env python3
"""Run the public-router development harness against a pinned prebuilt Core library.

Requires a completed freeze and a delegated cgroup. Never compiles the Core library.
All databases, roles and sockets are created solely inside a fresh private cluster.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
from urllib.parse import urlencode


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_pins(pins, prefix=None):
    for name, expected in pins.items():
        file = Path(name) if prefix is None else prefix / name
        if digest(file) != expected:
            raise RuntimeError(f"pinned input drift: {name}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--freeze", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--core-sha", required=True)
    parser.add_argument("--library-sha", required=True)
    parser.add_argument("--source-pins-sha", required=True)
    args = parser.parse_args()
    os.umask(0o077)
    freeze = args.freeze.resolve(strict=True)
    output = args.output.absolute()
    evidence_root = (Path.home() / "Documents/Codex").resolve(strict=True)
    if output.parent.resolve(strict=True) != evidence_root or output.exists():
        raise SystemExit("output must be a new direct child of Documents/Codex")
    cgroup = next(line.split(":", 2)[2] for line in Path("/proc/self/cgroup").read_text().splitlines() if line.startswith("0::"))
    control = Path("/sys/fs/cgroup") / cgroup.lstrip("/")
    limits = {name: (control / name).read_text().strip() for name in ("memory.max", "memory.high", "memory.swap.max", "cpu.max")}
    required = {"memory.max": "2147483648", "memory.high": "1610612736", "memory.swap.max": "4294967296", "cpu.max": "100000 100000"}
    if limits != required:
        raise SystemExit(f"resource gate rejected: {limits}")
    available = int(next(line.split()[1] for line in Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemAvailable:"))) * 1024
    if available < 2 * 1024**3:
        raise SystemExit("less than 2 GiB available")
    output.mkdir(mode=0o700)
    records = []
    started = False
    socket_dir = None
    passed = False
    cluster = output / "pg"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("PG", "LMM_", "PAYMENT_", "SQLX_", "CARGO_", "RUSTFLAGS")) and k not in ("DATABASE_URL", "SQL_DSN")}
    env.update(PATH="/usr/bin:/bin", LANG="C", TZ="UTC", TMPDIR=str(output))

    def save(name, value):
        (output / name).write_text(json.dumps(value, indent=2) + "\n")

    def run(label, argv, body=None, timeout=300):
        log_path = output / f"{len(records):03d}-{label}.log"
        entry = {"label": label, "argv": [str(x) for x in argv], "started": time.time()}
        records.append(entry)
        try:
            with log_path.open("xb") as log:
                result = subprocess.run(entry["argv"], input=body, stdout=log, stderr=subprocess.STDOUT, env=env, timeout=timeout)
            entry["exit"] = result.returncode
            if result.returncode:
                raise RuntimeError(f"{label} failed; see {log_path.name}")
        finally:
            entry["finished"] = time.time()
            save("commands.json", records)

    with open("/tmp/lmm-delegate/heavy-build.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            cfg_path = freeze / "compile-config.json"
            pins_path = freeze / "source-pins.json"
            cache_path = freeze / "cache-pins.json"
            library = freeze / "liblmm_core.rlib"
            core = freeze / "lmm-core"
            if digest(core) != args.core_sha or digest(library) != args.library_sha or digest(pins_path) != args.source_pins_sha:
                raise RuntimeError("freeze hash mismatch")
            config = json.loads(cfg_path.read_text())
            source_pins = json.loads(pins_path.read_text())
            cache_pins = json.loads(cache_path.read_text())
            verify_pins(source_pins, freeze / "source")
            verify_pins(cache_pins)
            result = json.loads((freeze / "result.json").read_text())
            if result.get("status") != "passed" or result.get("source_unchanged") is not True:
                raise RuntimeError("freeze build did not pass")
            commands = json.loads((freeze / "commands.json").read_text())
            by_name = {}
            for name in ("dev-library", "dev-main", "clippy-library"):
                matching = [record for record in commands if record.get("name") == name]
                if len(matching) != 1 or matching[0].get("exit_code") != 0 or matching[0].get("status") != "terminal":
                    raise RuntimeError(f"missing successful frozen build command: {name}")
                by_name[name] = matching[0]["argv"]
            if (by_name["dev-library"][0] != config["compiler"]
                or str(freeze / "source/services/core/src/lib.rs") not in by_name["dev-library"]
                or str(library) not in by_name["dev-library"]
                or f"lmm_core={library}" not in by_name["dev-main"]
                or str(core) not in by_name["dev-main"]):
                raise RuntimeError("frozen build commands do not bind source, library and executable")
            here = Path(__file__).resolve().parent
            source = freeze / "source/services/core/src/channel_http.rs"
            if digest(source) != digest(here.parent / "channel_http.rs"):
                raise RuntimeError("tested channel source is not the delivered patch")
            frozen_tests = freeze / "source/services/core/src/channel_http/authorization_tests.rs"
            if not frozen_tests.is_file() or digest(frozen_tests) != digest(here / "authorization_tests.rs"):
                raise RuntimeError("frozen cfg(test) source differs")
            permission_source = (freeze / "source/services/core/src/identity/permissions.rs").read_text()
            if "pub(crate) fn catalog_decision(" not in permission_source:
                raise RuntimeError("shared permission integration is missing")
            harness = here / "authorization_harness.rs"
            binding = {"scope": "development public Router with actual frozen Core library and isolated PG; not native CLI or independent acceptance", "core_sha256": digest(core), "library_sha256": digest(library), "source_pins_sha256": digest(pins_path), "cache_pins_sha256": digest(cache_path), "compile_config_sha256": digest(cfg_path), "build_commands_sha256": digest(freeze / "commands.json"), "harness_sha256": digest(harness), "runner_sha256": digest(__file__), "limits": limits}
            save("binding.json", binding)
            run("pg-version", ["initdb", "--version"])
            if not (output / "000-pg-version.log").read_text().startswith("initdb (PostgreSQL) 18."):
                raise RuntimeError("PostgreSQL 18 required")
            socket_dir = Path(tempfile.mkdtemp(prefix="lmm-channel-", dir="/tmp"))
            socket_dir.chmod(0o700)
            run("initdb", ["initdb", "-D", cluster, "-U", "postgres", "--encoding=UTF8", "--no-locale", "--auth=trust", "--wal-segsize=1"])
            options = f"-c listen_addresses= -c unix_socket_directories={socket_dir} -c unix_socket_permissions=0700 -p 55601 -c shared_buffers=16MB -c max_connections=20 -c min_wal_size=2MB -c max_wal_size=128MB"
            run("pg-start", ["pg_ctl", "-D", cluster, "-l", output / "postgres.log", "-o", options, "-w", "start"])
            started = True
            run("create-database", ["psql", "-Xq", "-v", "ON_ERROR_STOP=1", "-h", socket_dir, "-p", "55601", "-U", "postgres", "-d", "postgres"], b"CREATE DATABASE lmm_channel_fixture;\n")
            url = "postgresql://postgres@localhost/lmm_channel_fixture?" + urlencode({"host": str(socket_dir), "port": 55601, "sslmode": "disable"})
            url_file = output / "owner.url"
            url_file.write_text(url)
            env.update(LMM_CHANNEL_OWNER_URL_FILE=str(url_file), LMM_CHANNEL_PRIVATE_SOCKET=str(socket_dir), OUT_DIR=config["out_dir"], CARGO_MANIFEST_DIR=str(freeze / "source/services/core"), CARGO_PKG_VERSION="0.1.0")
            exe = output / "channel-authorization-test"
            run("compile-harness", [config["compiler"], "--edition=2024", "--test", "--crate-name", "channel_authorization_harness", harness, *config["search"], *config["externs"], *config["link"], "--extern", f"lmm_core={library}", "-o", exe], timeout=600)
            save("harness-binary.json", {"sha256": digest(exe)})
            run("actual-frozen-library-router-pg", [exe, "--test-threads=1", "--nocapture"], timeout=300)
            verify_pins(source_pins, freeze / "source")
            verify_pins(cache_pins)
            unchanged = {"core_sha256": digest(core), "library_sha256": digest(library),
                         "source_pins_sha256": digest(pins_path), "cache_pins_sha256": digest(cache_path),
                         "compile_config_sha256": digest(cfg_path), "build_commands_sha256": digest(freeze / "commands.json"),
                         "harness_sha256": digest(harness), "runner_sha256": digest(__file__)}
            if any(binding[key] != value for key, value in unchanged.items()):
                raise RuntimeError("bound test inputs changed during execution")
            passed = True
        finally:
            stop_error = None
            if started or (cluster / "postmaster.pid").exists():
                try:
                    run("pg-stop", ["pg_ctl", "-D", cluster, "-m", "fast", "-w", "stop"], timeout=90)
                    started = False
                except Exception as error:
                    passed = False
                    stop_error = error
            if socket_dir is not None and not started and stop_error is None:
                shutil.rmtree(socket_dir)
            if passed:
                shutil.rmtree(cluster)
                (output / "owner.url").unlink()
            save("result.json", {"status": "PASS" if passed else "FAILED_OR_BLOCKED", "scope": "development frozen Core library Router + synthetic PostgreSQL", "pgdata_cleaned": passed, "native_http": "NOT_RUN", "cfg_test_leaf": "NOT_RUN", "independent_acceptance": "NOT_RUN"})
            if stop_error is not None:
                raise stop_error


if __name__ == "__main__":
    main()
