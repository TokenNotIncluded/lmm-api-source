#!/usr/bin/env python3
"""Direct synthetic live process test; not the formal release-artifact runner.

Allocates only a fresh owned PG fixture. Credentials and sockets are ephemeral;
failed tests retain hashes and bounded redacted status, never raw private logs.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("payment_process", REPO / "tests/payments/verify_process.py")
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
CASE = "synthetic_live_proof_crosses_core_go_processes_and_posts_once"
BOOT = "TestSyntheticLiveProofAcrossCoreProcess"


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def failure_location(text):
    """Keep only a known test source location, never panic values or private logs."""
    match = re.search(
        r"panicked at (tests/payments_live_process\.rs):([1-9][0-9]{0,5}):([1-9][0-9]{0,5}):",
        text,
    )
    if match is None:
        return None
    return {"file": match[1], "line": int(match[2]), "column": int(match[3])}


def redacted_log_diagnostics(data):
    """Preserve fixed error categories and hashes, never raw process/SQL text."""
    text = data.decode("utf-8", errors="replace")
    markers = {
        "core_missing_privileges": "Error: MissingPrivileges",
        "core_unsafe_privileges": "Error: UnsafePrivileges",
        "core_unavailable": "Error: Unavailable",
        "payment_storage_contract_unavailable": "payment storage contract unavailable",
        "database_permission_denied": "permission denied for ",
    }
    categories = [name for name, marker in markers.items() if marker in text]
    for schema in ("core_payments", "core_subscriptions", "core_checkins", "core_gift"):
        if re.search(r"permission denied for schema " + schema + r"(?:\s|$)", text):
            categories.append("database_schema_permission_denied:" + schema)
    statements = re.findall(r"STATEMENT:  ([^\n]*(?:\n\t[^\n]*)*)", text)
    return {
        "observed_bytes": len(data),
        "observed_sha256": hashlib.sha256(data).hexdigest(),
        "categories": sorted(categories),
        "statement_sha256": sorted({hashlib.sha256(value.encode()).hexdigest() for value in statements}),
    }


def product_startup_diagnostics(root):
    result = {}
    for name in ("core-initial.log", "go-main.log"):
        matches = list(root.glob("pay-*/" + name))
        if len(matches) > 1:
            raise RuntimeError("ambiguous product log")
        if not matches:
            continue
        path = matches[0]
        if path.parent.is_symlink() or path.is_symlink() or not path.is_file():
            raise RuntimeError("unexpected product log type")
        with path.open("rb") as stream:
            stream.seek(max(0, path.stat().st_size - 65536))
            item = redacted_log_diagnostics(stream.read())
        item.update(log_bytes=path.stat().st_size, log_sha256=sha(path))
        item["saved_identity_stages"] = {}
        for stage in ("spawn", "before-ready", "ready", "final"):
            receipt = path.with_name(name + ".identity." + stage + ".json")
            if receipt.exists():
                if receipt.is_symlink() or not receipt.is_file():
                    raise RuntimeError("unexpected identity receipt type")
                item["saved_identity_stages"][stage] = sha(receipt)
        result[name] = item
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compiled-inputs", type=Path, required=True)
    for kind in ("core", "go", "test", "bootstrap"):
        parser.add_argument("--" + kind + "-binary", type=Path, required=True)
        parser.add_argument("--" + kind + "-sha256", required=True)
    args = parser.parse_args()
    output = args.output
    if not output.is_absolute() or output.exists() or output.parent.resolve() != output.parent:
        parser.error("output must be a new absolute directory without symlink parents")
    output.mkdir(mode=0o700)
    frozen = v.sources()
    if json.loads(args.compiled_inputs.read_text()) != frozen:
        parser.error("compiled inputs differ from current frozen sources")
    binaries = {}
    for kind in ("core", "go", "test", "bootstrap"):
        path = getattr(args, kind + "_binary")
        expected = getattr(args, kind + "_sha256")
        if not path.is_absolute() or path.is_symlink() or not path.is_file() or sha(path) != expected:
            parser.error("sealed artifact mismatch: " + kind)
        binaries[kind] = {"path": str(path), "sha256": expected}
    v.write_private(output / "inputs.json", json.dumps({"source_sha256": frozen, "binaries": binaries}))
    root = None
    root_identity = None
    marker = None
    fixture = None
    child = None
    status = {"result": "FAIL", "scope": "synthetic live only; formal artifact preflight not satisfied"}
    started = time.monotonic()
    # Keep fixture ownership through interrupt so finally can reap clients first.
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        root = Path(tempfile.mkdtemp(prefix="sl-", dir="/tmp"))
        metadata = root.lstat()
        root_identity = (metadata.st_dev, metadata.st_ino, metadata.st_uid)
        root.chmod(0o700)
        marker = {"runner_pid": os.getpid(), "root": str(root), "nonce": os.urandom(16).hex()}
        # Fixture construction creates only private metadata; no PG exists yet.
        fixture = v.Fixture(output)
        env = {k: val for k, val in v.clean_environment().items() if not k.startswith(("LMM_", "SQLX_", "PAYMENT_"))}
        env.update(TMPDIR=str(root), LMM_LIVE_PROCESS_ROOT=str(root))
        listed = subprocess.run([binaries["test"]["path"], "--list", "--exact", CASE], env=env, capture_output=True, text=True, timeout=10, check=True)
        if listed.stdout.strip() != CASE + ": test\n\n1 test, 0 benchmarks":
            raise RuntimeError("unexpected Rust test discovery")
        listed = subprocess.run([binaries["bootstrap"]["path"], "-test.list", "^" + BOOT + "$"], env=env, capture_output=True, text=True, timeout=10, check=True)
        if listed.stdout.strip() != BOOT:
            raise RuntimeError("unexpected bootstrap discovery")
        fixture.start()
        marker.update(port=fixture.binding["port"], system_id=fixture.identity["system_id"], postmaster_start=fixture.identity["postmaster_start"])
        v.write_private(root / "owner.json", json.dumps(marker))
        config = dict(line.split("=", 1) for line in (fixture.root / "postgres.env").read_text().splitlines())
        env["DATABASE_URL"] = f"postgresql://a09_admin:{config['POSTGRES_PASSWORD']}@127.0.0.1:{fixture.binding['port']}/postgres"
        for kind in ("core", "go", "bootstrap"):
            env["LMM_PAYMENT_" + kind.upper() + "_BIN"] = binaries[kind]["path"]
            env["LMM_PAYMENT_" + kind.upper() + "_SHA256"] = binaries[kind]["sha256"]
        log = root / "test.log"
        with log.open("xb") as stream:
            log.chmod(0o600)
            child = subprocess.Popen([binaries["test"]["path"], CASE, "--exact", "--nocapture", "--test-threads=1"], cwd=REPO / "services/core", env=env, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            code = child.wait(timeout=180)
        text = log.read_text()
        status.update(exit_code=code, log_sha256=sha(log))
        if code != 0:
            status["failure_location"] = failure_location(text)
        if code != 0 or "1 passed; 0 failed; 0 ignored;" not in text:
            raise RuntimeError("selected live test failed or did not execute")
        results = list(root.glob("pay-*/result.json"))
        if len(results) != 1:
            raise RuntimeError("missing unique result")
        result = json.loads(results[0].read_text())
        if result.get("result") != "PASS" or result.get("mode") != "synthetic-live" or not result.get("products_reaped"):
            raise RuntimeError("incomplete process evidence")
        if v.sources() != frozen or any(sha(Path(item["path"])) != item["sha256"] for item in binaries.values()):
            raise RuntimeError("frozen inputs changed")
        status.update(result="PASS", process_result=result)
    except (Exception, KeyboardInterrupt) as error:
        # Exception messages and raw logs may contain database URLs or tokens.
        status.update(error_type=type(error).__name__)
    finally:
        # The process group contains only this freshly launched test and products.
        # Reap any surviving child products on test failure before PG teardown.
        if child is not None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        def cleanup_error(step, error):
            status["result"] = "FAIL"
            status.setdefault("cleanup_errors", {})[step] = type(error).__name__

        root_owned = False
        try:
            if root is not None:
                meta = root.lstat()
                root_owned = (meta.st_dev, meta.st_ino, meta.st_uid) == root_identity and not root.is_symlink() and meta.st_uid == os.getuid()
                if not root_owned:
                    raise RuntimeError("ephemeral root identity changed")
        except Exception as error:
            cleanup_error("root_identity", error)
        # Optional evidence must never gate cleanup of owned PG or secrets.
        try:
            if root_owned:
                evidence = output / "assertions"
                evidence.mkdir(mode=0o700)
                log = root / "test.log"
                if log.is_file() and not log.is_symlink():
                    status["log_sha256"] = sha(log)
                if status["result"] != "PASS":
                    status["product_startup_diagnostics"] = product_startup_diagnostics(root)
                for pattern in ("pay-*/result.json", "pay-*/main-live-credit.json", "pay-*/*.identity.*.json", "pay-*/bootstrap-*-exit.json"):
                    for path in root.glob(pattern):
                        if path.is_symlink() or not path.is_file():
                            raise RuntimeError("unexpected evidence type")
                        v.write_private(evidence / path.name, path.read_text())
        except Exception as error:
            cleanup_error("evidence", error)
        cluster_gone = fixture is None or fixture.container is None
        try:
            if fixture is not None and fixture.container is not None:
                # UNKNOWN provisioning is retained; never infer missing identity.
                fixture.verify()
                if status["result"] != "PASS":
                    try:
                        logs = fixture.docker("logs", "--tail", "100", fixture.container, timeout=15)
                        status["postgres_failure_diagnostics"] = redacted_log_diagnostics(logs.stdout + logs.stderr)
                    except Exception as error:
                        # Optional diagnostics must not block verified fixture cleanup.
                        status["postgres_diagnostics_error_type"] = type(error).__name__
                fixture.cleanup()
                cluster_gone = True
            status["cluster_removed_or_not_created"] = cluster_gone
        except Exception as error:
            cleanup_error("postgres", error)
        # If PG identity is UNKNOWN, preserve private recovery material.
        try:
            if cluster_gone and root_owned:
                shutil.rmtree(root)
                status["private_root_removed"] = not root.exists()
            elif root is not None:
                status["retained_private_root"] = str(root)
        except Exception as error:
            cleanup_error("private_root", error)
        try:
            if cluster_gone and fixture is not None:
                password_file = fixture.root / "postgres.env"
                if password_file.exists():
                    password_file.unlink()
        except Exception as error:
            cleanup_error("password_file", error)
        status["seconds"] = round(time.monotonic() - started, 3)
        v.write_private(output / "result.json", json.dumps(status, sort_keys=True))
    print(status["result"] + ": synthetic live process; receipt " + str(output / "result.json"))
    return 0 if status["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
