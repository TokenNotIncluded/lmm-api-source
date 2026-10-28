#!/usr/bin/env python3
"""Exercise a PostgreSQL connection fence on a new, disposable local cluster.

This is a synthetic experiment, not a site maintenance command. It cannot take
an existing database URL and does not prove that other site writers are fenced.
"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def main():
    root = Path(tempfile.mkdtemp(prefix="lmm-fence-synthetic-", dir="/tmp"))
    data, socket = root / "data", root / "socket"
    socket.mkdir(mode=0o700)
    env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    env["LC_ALL"] = "C"
    started = False
    writer = None
    passed = False

    def run(args, *, check=True, timeout=60, environment=env):
        return subprocess.run(args, env=environment, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=timeout, check=check)

    def sql(database, statement, *, user="fence_admin", check=True):
        return run(["psql", "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose",
                    "-h", str(socket), "-p", "55476", "-U", user,
                    "-d", database, "-c", statement], check=check)

    def assert_fenced_connection():
        result = sql("fence_synthetic", "SELECT 1", check=False)
        if (result.returncode == 0 or
                'database "fence_synthetic" is not currently accepting connections' not in result.stderr):
            raise RuntimeError("database did not reject the connection for the expected reason")
        if sql("postgres", "SELECT datallowconn FROM pg_database WHERE "
               "datname='fence_synthetic'").stdout.strip() != "f":
            raise RuntimeError("database connection fence was not set")

    try:
        run(["initdb", "-D", str(data), "-U", "fence_admin", "--auth-local=trust",
             "--auth-host=reject", "--no-locale", "--encoding=UTF8"])
        options = f"-c listen_addresses='' -c unix_socket_directories={socket} -p 55476"
        start = ["pg_ctl", "-D", str(data), "-l", str(root / "postgres.log"),
                 "-o", options, "-w", "start"]
        run(start)
        started = True
        sql("postgres", "CREATE ROLE fence_writer LOGIN")
        sql("postgres", "CREATE DATABASE fence_synthetic")
        sql("fence_synthetic", "CREATE TABLE fence_probe(id integer); INSERT INTO fence_probe VALUES(7); "
            "GRANT INSERT ON fence_probe TO fence_writer")
        writer = subprocess.Popen(
            ["psql", "-X", "-v", "ON_ERROR_STOP=1", "-h", str(socket), "-p", "55476",
             "-U", "fence_writer", "-d", "fence_synthetic", "-c",
             "BEGIN; INSERT INTO fence_probe VALUES(1); SELECT pg_sleep(120); COMMIT"],
            env={**env, "PGAPPNAME": "synthetic_fence_writer"}, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            active = sql("postgres", "SELECT count(*) FROM pg_stat_activity WHERE "
                         "datname='fence_synthetic' AND application_name='synthetic_fence_writer' "
                         "AND state='active' AND wait_event='PgSleep'").stdout.strip()
            if active == "1":
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("synthetic writer did not complete its insert before sleeping")

        sql("postgres", "ALTER DATABASE fence_synthetic ALLOW_CONNECTIONS false")
        terminated = sql("postgres", "SELECT count(*) FROM (SELECT pg_terminate_backend(pid) AS ok "
                         "FROM pg_stat_activity WHERE datname='fence_synthetic' AND "
                         "application_name='synthetic_fence_writer') t WHERE ok").stdout.strip()
        if terminated != "1":
            raise RuntimeError("active writer was not terminated")
        writer.communicate(timeout=10)
        if writer.returncode == 0:
            raise RuntimeError("fenced writer unexpectedly committed")
        assert_fenced_connection()

        run(["pg_ctl", "-D", str(data), "-m", "immediate", "-w", "stop"])
        started = False
        run(start)
        started = True
        if sql("postgres", "SELECT datallowconn FROM pg_database WHERE "
               "datname='fence_synthetic'").stdout.strip() != "f":
            raise RuntimeError("fence did not survive crash restart")
        assert_fenced_connection()
        sql("postgres", "ALTER DATABASE fence_synthetic ALLOW_CONNECTIONS true")
        if sql("fence_synthetic", "SELECT count(*),sum(id) FROM fence_probe").stdout.strip() != "1|7":
            raise RuntimeError("terminated writer changed committed data")
        passed = True
    finally:
        if writer is not None and writer.poll() is None:
            writer.terminate()
            try:
                writer.wait(timeout=5)
            except subprocess.TimeoutExpired:
                writer.kill()
                writer.wait()
        if started:
            stopped = run(["pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop"], check=False)
            if stopped.returncode != 0:
                passed = False
                raise RuntimeError(f"synthetic cluster stop failed; fixture retained: {root}")
        if passed:
            shutil.rmtree(root)
        else:
            print("Synthetic failure fixture retained:", root)

    print("PASS: synthetic writer rolled back; connection fence survived crash restart; "
          "manual reopening verified")


if __name__ == "__main__":
    main()
