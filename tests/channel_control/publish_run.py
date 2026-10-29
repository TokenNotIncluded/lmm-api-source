#!/usr/bin/env python3
"""Run the compiled harness against a fresh socket-only PostgreSQL instance."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from urllib.parse import quote


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path, help="publish_compile.py output")
    args = parser.parse_args()
    out = args.out.resolve(strict=True)
    binary = out / "publish-harness"
    compiled = json.loads((out / "compile-result.json").read_text(encoding="utf-8"))
    if (
        compiled.get("compile_exit") != 0
        or compiled.get("binding_intact") is not True
        or sha(binary) != compiled.get("binary_sha256")
    ):
        raise SystemExit("compiled harness is missing or does not match compile evidence")
    for name in ("test.log", "postgres.log", "test-result.json"):
        if (out / name).exists():
            raise SystemExit(f"refusing to overwrite prior test evidence: {out / name}")

    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("PG", "SQLX_")) and key not in ("DATABASE_URL", "SQL_DSN")
    }
    root = Path(tempfile.mkdtemp(prefix="lmm-channel-pub-", dir="/tmp"))
    data = root / "data"
    socket = root / "socket"
    socket.mkdir(mode=0o700)
    if len(os.fsencode(socket / ".s.PGSQL.5432")) > 107:
        raise SystemExit(f"Unix socket path is too long: {socket}")
    log_path = root / "postgres.log"
    result = {"binary_sha256": compiled["binary_sha256"], "pg_root": str(root)}
    started = False
    stopped = False
    passed = False
    phase = "initdb"
    try:
        with (out / "test.log").open("xb") as log:
            def run(command: list[str], timeout: int) -> int:
                log.write(("$ " + " ".join(command) + "\n").encode())
                log.flush()
                return subprocess.run(
                    command, env=env, stdout=log, stderr=subprocess.STDOUT,
                    timeout=timeout, check=False,
                ).returncode

            code = run([
                "initdb", "-D", str(data), "-U", "publish_test_admin",
                "--auth-local=trust", "--auth-host=reject", "--no-instructions",
            ], 120)
            if code != 0:
                raise RuntimeError(f"initdb exited {code}")
            phase = "start"
            options = (
                f"-c listen_addresses='' -c unix_socket_directories={socket} "
                "-c shared_buffers=16MB -c max_connections=24"
            )
            code = run(["pg_ctl", "-D", str(data), "-l", str(log_path),
                        "-o", options, "-w", "start"], 120)
            if code != 0:
                raise RuntimeError(f"pg_ctl start exited {code}")
            started = True
            phase = "tests"
            env["DATABASE_URL"] = (
                "postgresql://publish_test_admin@localhost/postgres?host="
                + quote(str(socket), safe="")
            )
            code = run([str(binary), "--test-threads=1", "--nocapture"], 180)
            result["test_exit"] = code
            passed = code == 0
            if not passed:
                raise RuntimeError(f"test harness exited {code}")
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        result["error"] = str(error)
    finally:
        if started or (data / "postmaster.pid").exists():
            for mode in ("fast", "immediate"):
                try:
                    stop = subprocess.run(
                        ["pg_ctl", "-D", str(data), "-m", mode, "-w", "stop"],
                        env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                        timeout=120, check=False,
                    )
                    result[f"stop_{mode}_exit"] = stop.returncode
                    stopped = stop.returncode == 0
                    if not stopped:
                        result[f"stop_{mode}_output"] = stop.stdout.decode(
                            "utf-8", errors="replace"
                        )
                except (OSError, subprocess.TimeoutExpired) as error:
                    result[f"stop_{mode}_error"] = str(error)
                if stopped:
                    break
        if log_path.exists():
            shutil.copyfile(log_path, out / "postgres.log")
        result.update({"phase": phase, "passed": passed, "stopped": stopped})
        if passed and stopped:
            shutil.rmtree(root)
            result["pg_root_cleaned"] = True
        else:
            result["pg_root_cleaned"] = False
        (out / "test-result.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
    print(json.dumps(result, ensure_ascii=False))
    if not passed or not stopped:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
