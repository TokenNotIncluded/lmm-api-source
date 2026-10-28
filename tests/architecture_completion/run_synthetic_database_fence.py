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

    def assert_fenced_connection(database):
        result = sql(database, "SELECT 1", check=False)
        if (result.returncode == 0 or
                f'database "{database}" is not currently accepting connections' not in result.stderr):
            raise RuntimeError("database did not reject the connection for the expected reason")
        if sql("postgres", "SELECT datallowconn FROM pg_database WHERE "
               f"datname='{database}'").stdout.strip() != "f":
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
        assert_fenced_connection("fence_synthetic")

        run(["pg_ctl", "-D", str(data), "-m", "immediate", "-w", "stop"])
        started = False
        run(start)
        started = True
        if sql("postgres", "SELECT datallowconn FROM pg_database WHERE "
               "datname='fence_synthetic'").stdout.strip() != "f":
            raise RuntimeError("fence did not survive crash restart")
        assert_fenced_connection("fence_synthetic")
        sql("postgres", "ALTER DATABASE fence_synthetic ALLOW_CONNECTIONS true")
        if sql("fence_synthetic", "SELECT count(*),sum(id) FROM fence_probe").stdout.strip() != "1|7":
            raise RuntimeError("terminated writer changed committed data")

        # Separately crash while the uncommitted writer is still running.
        sql("postgres", "CREATE DATABASE fence_crash")
        sql("fence_crash", "CREATE TABLE fence_probe(id integer); INSERT INTO fence_probe VALUES(7); "
            "GRANT INSERT ON fence_probe TO fence_writer")
        writer = subprocess.Popen(
            ["psql", "-X", "-v", "ON_ERROR_STOP=1", "-h", str(socket), "-p", "55476",
             "-U", "fence_writer", "-d", "fence_crash", "-c",
             "BEGIN; INSERT INTO fence_probe VALUES(1); SELECT pg_sleep(120); COMMIT"],
            env={**env, "PGAPPNAME": "synthetic_crash_writer"}, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            active = sql("postgres", "SELECT count(*) FROM pg_stat_activity WHERE "
                         "datname='fence_crash' AND application_name='synthetic_crash_writer' "
                         "AND state='active' AND wait_event='PgSleep'").stdout.strip()
            if active == "1":
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("crash writer did not complete its insert before sleeping")
        sql("postgres", "ALTER DATABASE fence_crash ALLOW_CONNECTIONS false")
        assert_fenced_connection("fence_crash")
        run(["pg_ctl", "-D", str(data), "-m", "immediate", "-w", "stop"])
        started = False
        writer.communicate(timeout=10)
        if writer.returncode == 0:
            raise RuntimeError("crashed writer unexpectedly committed")
        run(start)
        started = True
        assert_fenced_connection("fence_crash")
        sql("postgres", "ALTER DATABASE fence_crash ALLOW_CONNECTIONS true")
        if sql("fence_crash", "SELECT count(*),sum(id) FROM fence_probe").stdout.strip() != "1|7":
            raise RuntimeError("crashed writer changed committed data")

        # A distinct importer can reconnect while the old login stays fenced.
        sql("postgres", "CREATE ROLE fence_legacy LOGIN; CREATE ROLE fence_importer LOGIN")
        sql("postgres", "CREATE DATABASE fence_migration")
        sql("postgres", "REVOKE ALL ON DATABASE fence_migration FROM PUBLIC; "
            "GRANT CONNECT ON DATABASE fence_migration TO fence_legacy,fence_importer")
        sql("fence_migration", "CREATE TABLE fence_probe(id integer); INSERT INTO fence_probe VALUES(7); "
            "GRANT INSERT ON fence_probe TO fence_legacy; GRANT SELECT ON fence_probe TO fence_importer")
        writer = subprocess.Popen(
            ["psql", "-X", "-v", "ON_ERROR_STOP=1", "-h", str(socket), "-p", "55476",
             "-U", "fence_legacy", "-d", "fence_migration", "-c",
             "BEGIN; INSERT INTO fence_probe VALUES(1); SELECT pg_sleep(120); COMMIT"],
            env={**env, "PGAPPNAME": "synthetic_legacy_writer"}, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            active = sql("postgres", "SELECT count(*) FROM pg_stat_activity WHERE "
                         "datname='fence_migration' AND application_name='synthetic_legacy_writer' "
                         "AND state='active' AND wait_event='PgSleep'").stdout.strip()
            if active == "1":
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("legacy writer did not complete its insert before sleeping")
        sql("postgres", "ALTER ROLE fence_legacy NOLOGIN")
        if writer.poll() is not None:
            raise RuntimeError("legacy writer unexpectedly ended before session drain")
        terminated = sql("postgres", "SELECT count(*) FROM (SELECT pg_terminate_backend(pid,5000) AS ok "
                         "FROM pg_stat_activity WHERE datname='fence_migration' AND "
                         "application_name='synthetic_legacy_writer') t WHERE ok").stdout.strip()
        if terminated != "1":
            raise RuntimeError("legacy writer did not drain")
        writer.communicate(timeout=10)
        if writer.returncode == 0:
            raise RuntimeError("drained legacy writer unexpectedly committed")

        def check_migration_access():
            old = sql("fence_migration", "SELECT 1", user="fence_legacy", check=False)
            if old.returncode == 0 or 'role "fence_legacy" is not permitted to log in' not in old.stderr:
                raise RuntimeError("legacy login was not rejected for the expected reason")
            if sql("fence_migration", "SELECT count(*),sum(id) FROM fence_probe",
                   user="fence_importer").stdout.strip() != "1|7":
                raise RuntimeError("importer cannot read the preserved committed data")

        check_migration_access()
        run(["pg_ctl", "-D", str(data), "-m", "immediate", "-w", "stop"])
        started = False
        run(start)
        started = True
        check_migration_access()

        # NOLOGIN does not prevent a different login from assuming this role.
        sql("postgres", "CREATE ROLE fence_alias LOGIN NOINHERIT; "
            "GRANT fence_legacy TO fence_alias WITH INHERIT FALSE, SET TRUE; "
            "GRANT CONNECT ON DATABASE fence_migration TO fence_alias")
        assumed = sql("fence_migration", "SET ROLE fence_legacy; "
                      "SELECT has_table_privilege(current_user,'public.fence_probe','INSERT')",
                      user="fence_alias")
        if assumed.stdout.strip().splitlines()[-1] != "t":
            raise RuntimeError("legacy role membership bypass was not reproduced")
        sql("postgres", "REVOKE fence_legacy FROM fence_alias")
        denied = sql("fence_migration", "SET ROLE fence_legacy", user="fence_alias", check=False)
        if denied.returncode == 0 or "42501" not in denied.stderr:
            raise RuntimeError("revoked legacy role was still assumable")
        check_migration_access()
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

    print("PASS: terminated and crashed writers rolled back; connection and legacy-login fences "
          "survived crash restarts; importer access and role-membership risk verified")


if __name__ == "__main__":
    main()
