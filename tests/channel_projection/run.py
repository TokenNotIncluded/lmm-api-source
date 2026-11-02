#!/usr/bin/env python3
"""Fixed local acceptance runner. Default is a read-only plan; never builds.
Execution requires Main's private approval binding prebuilt binaries, sources,
and an empty existing isolated PG database. No production endpoints or PG tuning.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path.home() / ".cache"
IMAGE = "usage::channel_projection::image_acceptance::channel_projection_image_real_funding_replay_and_owner_rebuild"
TEST = "channel_projection_acceptance"
MIN_FREE = 32 * 1024**3


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def source_digest():
    files = set((ROOT / "services/core/src").rglob("*.rs"))
    files.update((ROOT / "services/core/schema").rglob("*.sql"))
    files.update((ROOT / "services/core/tests").rglob("*.rs"))
    files.update((ROOT / "tests/channel_projection").glob("*.py"))
    files.update((ROOT / "tests/channel_projection").glob("*.md"))
    for name in ("services/core/Cargo.toml", "services/core/Cargo.lock", "deployment/postgres/grant-core-runtime.sql"):
        files.add(ROOT / name)
    h = hashlib.sha256()
    for path in sorted(files):
        h.update(path.relative_to(ROOT).as_posix().encode() + b"\0")
        h.update(path.read_bytes())
    return h.hexdigest(), len(files)


def cached(path):
    result = Path(path).expanduser().resolve(strict=True)
    if not result.is_relative_to(CACHE.resolve()):
        raise ValueError("all run artifacts must be inside home cache")
    return result


def private_json(path):
    p = cached(path)
    if p.stat().st_mode & 0o077:
        raise ValueError("approval must be private (mode 0600)")
    return json.loads(p.read_text())


def read_secret_file(path):
    p = cached(path)
    if p.stat().st_mode & 0o077:
        raise ValueError("connection file must be private (mode 0600)")
    return p.read_text().strip()


def space_check(output, pgdata):
    # Run immediately before data generation, even after an earlier plan/check.
    subprocess.run(["df", "-h", "--", str(output), str(pgdata)], check=True)
    for tool, argv in (("dust", ["-d", "1", str(output)]), ("prefixlens", [str(output), "--summary"])):
        if shutil.which(tool):
            result = subprocess.run(["timeout", "10s", "ionice", "-c", "3", tool, *argv], check=False)
            if result.returncode:
                print(f"{tool}: incomplete inventory (exit {result.returncode}); no cleanup inferred")
        else:
            print(f"{tool}: unavailable; bounded directory inventory used")
    print("existing runner artifacts:", sorted(p.name for p in output.iterdir())[:30])
    for path in (output, pgdata):
        if shutil.disk_usage(path).free < MIN_FREE:
            raise ValueError("million receipt/WAL/index budget requires 32 GiB free on each target filesystem")


# Runs inside the same cgroup as the test and waits for that exact child PID.
# Linux wait4 reports ru_maxrss in KiB; times use seconds, not CPU percentages.
RESOURCE_WORKER = """import json, os, subprocess, sys, time
from pathlib import Path
started = time.monotonic()
child = subprocess.Popen(sys.argv[2:])
_, status, usage = os.wait4(child.pid, 0)
child.returncode = os.waitstatus_to_exitcode(status)
Path(sys.argv[1]).write_text(json.dumps({
    'clock': 'time.monotonic',
    'time_unit': 'seconds',
    'max_rss_unit': 'KiB (Linux wait4 ru_maxrss)',
    'filesystem_io_unit': 'operations (wait4 ru_inblock/ru_oublock)',
    'wall_seconds': time.monotonic() - started,
    'user_cpu_seconds': usage.ru_utime,
    'system_cpu_seconds': usage.ru_stime,
    'max_rss_kib': usage.ru_maxrss,
    'filesystem_input_operations': usage.ru_inblock,
    'filesystem_output_operations': usage.ru_oublock,
    'exit_code': child.returncode
}, indent=2))
sys.exit(child.returncode if child.returncode >= 0 else 128 - child.returncode)
"""


def run_stage(name, binary, target, env, output):
    unit = f"lmm-channel-projection-{uuid.uuid4().hex[:12]}"
    log = output / f"{name}.log"
    resources = output / f"{name}.resources.json"
    start = time.monotonic()
    argv = ["systemd-run", "--user", "--scope", "--quiet", f"--unit={unit}",
            "-p", "MemoryHigh=4250M", "-p", "MemoryMax=5000M",
            "-p", "MemorySwapMax=1536M", "-p", "CPUQuota=150%",
            sys.executable, "-B", "-c", RESOURCE_WORKER, str(resources), str(binary),
            "--exact", target, "--nocapture", "--test-threads=1"]
    if name == "native-and-million":
        argv.append("--ignored")
    # Probe only the precompiled test inventory. A typo must not pass as zero tests.
    inventory = subprocess.run([str(binary), "--list"], capture_output=True, env=env, timeout=30, check=True).stdout.decode()
    if f"{target}: test" not in inventory.splitlines():
        raise ValueError(f"required compiled test absent: {target}")
    with log.open("xb") as stream:
        try:
            result = subprocess.run(argv, env=env, stdout=stream, stderr=subprocess.STDOUT,
                                    stdin=subprocess.DEVNULL, timeout=7200, check=False)
        except subprocess.TimeoutExpired:
            subprocess.run(["systemctl", "--user", "kill", "--kill-whom=all", f"{unit}.scope"], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            raise RuntimeError(f"{name} exceeded fixed 7200-second budget; preserve private fixture/log") from None
    text = log.read_text(errors="replace")
    if result.returncode or "test result: ok. 1 passed; 0 failed;" not in text:
        raise RuntimeError(f"{name} failed; private log: {log}")
    measurements = []
    for line in text.splitlines():
        marker = "PROJECTION_MEASURE "
        if marker in line:
            measurements.append(json.loads(line.split(marker, 1)[1]))
    return {"stage": name, "exit_code": result.returncode, "wall_seconds": time.monotonic() - start,
            "measurements": measurements, "resource_file": resources.name,
            "resources": json.loads(resources.read_text())}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--approval", type=Path)
    args = parser.parse_args()
    sha, count = source_digest()
    plan = {"source_files_sha256": sha, "source_file_count": count,
            "stages": [IMAGE, TEST], "scale_receipts": 1_000_010, "batch": 1000,
            "cgroup": {"memory_high": "4250M", "memory_max": "5000M", "swap_max": "1536M", "cpu": "150%"},
            "status": "PLAN_ONLY; no Cargo, PG, database creation or data generation"}
    if not args.execute:
        print(json.dumps(plan, indent=2))
        return
    if not args.approval:
        parser.error("--execute requires Main's private --approval receipt")
    os.umask(0o077)
    receipt = private_json(args.approval)
    if receipt.get("isolated_local") is not True or receipt.get("allow_image_sqlx_database") is not True:
        raise ValueError("Main must coordinate isolated fixture and image SQLx child database")
    if receipt["source_files_sha256"] != sha:
        raise ValueError("source changed since approval/binary freeze")
    binaries = {}
    for name in ("lib", "integration"):
        binary = cached(receipt[f"{name}_binary"])
        if digest(binary) != receipt[f"{name}_binary_sha256"]:
            raise ValueError("compiled artifact identity mismatch")
        binaries[name] = binary
    output_base = cached(receipt["output_directory"])
    pgdata = cached(receipt["pg_data_directory"])
    space_check(output_base, pgdata)
    output = output_base / f"run-{time.strftime('%Y%m%dT%H%M%S')}-{uuid.uuid4().hex[:8]}"
    output.mkdir(mode=0o700)
    env = os.environ.copy()
    env["DATABASE_URL"] = read_secret_file(receipt["image_database_url_file"])
    env["LMM_PROJECTION_DATABASE_URL"] = read_secret_file(receipt["projection_database_url_file"])
    env["LMM_PROJECTION_DATABASE_NAME"] = receipt["projection_database_name"]
    env["LMM_PROJECTION_ROWS"] = "1000000"
    results = []
    try:
        results.append(run_stage("real-image", binaries["lib"], IMAGE, env, output))
        # Recheck available space after image, directly before bulk-producing test.
        space_check(output_base, pgdata)
        results.append(run_stage("native-and-million", binaries["integration"], TEST, env, output))
    finally:
        (output / "result.json").write_text(json.dumps({**plan, "status": "PASS" if len(results) == 2 else "FAIL_OR_INCOMPLETE",
                                                       "binaries": {k: digest(v) for k, v in binaries.items()}, "results": results}, indent=2))
    print(f"PASS; private measurements: {output / 'result.json'}")
    # No fixture/database is dropped. Main owns database lifetime; failure evidence
    # and successful measurement artifacts remain reviewable inside home cache.


if __name__ == "__main__":
    main()
