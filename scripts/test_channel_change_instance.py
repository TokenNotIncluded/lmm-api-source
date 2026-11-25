#!/usr/bin/env python3
"""Exercise a file-managed channel release on disposable PostgreSQL 18 and two Core processes."""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
from tempfile import TemporaryDirectory
import time
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener
from test_channel_change import change, model_inputs


def run(argv, *, data=None, env=None, quiet=False):
    result = subprocess.run(argv, input=data, capture_output=True, env=env, timeout=180)
    if result.returncode:
        # The bootstrap command writes a one-time Session credential on success.
        detail = "" if quiet else result.stderr.decode(errors="replace")[-1024:]
        raise RuntimeError(f"{Path(argv[0]).name} failed ({result.returncode}) {detail}")
    return result.stdout


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def write(path, text):
    path.write_text(text)
    path.chmod(0o600)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pg-bin", required=True, type=Path, help="PostgreSQL 18 bin directory with initdb and pg_ctl")
    parser.add_argument("--psql", required=True, type=Path)
    parser.add_argument("--core", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path, help="Matching source tree with deployment/postgres SQL")
    parser.add_argument("--temp-root", type=Path, help="Filesystem for the disposable database")
    opts = parser.parse_args()
    os.umask(0o077)
    with TemporaryDirectory(dir=opts.temp_root, prefix="channel-instance-") as tmp, ExitStack() as cleanup:
        root = Path(tmp).resolve()
        pg = root / "pg"
        port, password, runtime_password = free_port(), secrets.token_urlsafe(32), secrets.token_urlsafe(32)
        pwfile = write(root / "pg.password", password + "\n")
        env = {**os.environ, "PGPASSWORD": password}
        run([opts.pg_bin / "initdb", "--no-sync", "--no-locale", "-E", "UTF8", "-D", pg, "-U", "postgres",
             "-A", "scram-sha-256", "--pwfile", pwfile])
        pgctl = [opts.pg_bin / "pg_ctl", "-D", pg]
        run(pgctl + ["-l", root / "postgres.log", "-o",
            f"-p {port} -c listen_addresses=127.0.0.1 -c unix_socket_directories='' -c shared_buffers=16MB -c max_connections=32",
            "-w", "start"])
        cleanup.callback(run, pgctl + ["-m", "fast", "-w", "stop"])
        psql = [opts.psql, "-Xq", "-h", "127.0.0.1", "-p", str(port), "-U", "postgres", "-v", "ON_ERROR_STOP=1"]
        pg_version = run(psql + ["-d", "postgres", "-Atc", "SHOW server_version"], env=env).decode().strip()
        assert pg_version.startswith("18."), pg_version
        run(psql + ["-d", "postgres", "-c", "CREATE DATABASE channel_fixture"], env=env)
        sql = opts.source / "deployment/postgres"
        psql += ["-d", "channel_fixture"]
        run(psql + ["-f", sql / "create-core-roles.sql"], env=env)
        run(psql, data=("ALTER DATABASE channel_fixture OWNER TO lmm_core_owner;\n"
             f"ALTER ROLE lmm_core_runtime PASSWORD '{runtime_password}';\n").encode(), env=env, quiet=True)
        owner_url = write(root / "owner.url", f"postgresql://postgres:{quote(password)}@127.0.0.1:{port}/channel_fixture?sslmode=disable&options=-c%20role%3Dlmm_core_owner")
        owner = write(root / "owner.toml", f"version=1\n[database]\nurl_file={json.dumps(str(owner_url))}\n")
        run([opts.core, "--config", owner, "init-db"])
        user = json.loads(run([opts.core, "--config", owner, "bootstrap-user"],
                           data=b'{"user_id":77,"platform_level":6}', quiet=True))
        session = write(root / "admin.session", user["secret"])
        run(psql + ["-v", "enable_models=true", "-v", "enable_console=true", "-f", sql / "grant-core-runtime.sql"], env=env)

        repo = root / "config"
        repo.mkdir()
        git = ["git", "-C", repo]
        run(git + ["init", "-q", "-b", "reviewed"])
        run(git + ["config", "user.name", "Fixture reviewer"])
        run(git + ["config", "user.email", "operator@example.invalid"])
        model, intent = model_inputs()
        for suffix in ("a", "b"):
            credential = write(root / f"channel-{suffix}.token", secrets.token_urlsafe(24))
            model = model.replace(f"/run/secrets/channel-{suffix}".encode(), str(credential).encode())
        configured = write(root / "installed.toml", model.decode())
        write(repo / "models.toml", model.decode())
        run(git + ["add", "models.toml"])
        run(git + ["commit", "-qm", "Reviewed base"])
        host_port = free_port()
        runtime_url = write(root / "runtime.url", f"postgresql://lmm_core_runtime:{quote(runtime_password)}@127.0.0.1:{port}/channel_fixture?sslmode=disable")
        host = write(root / "host.toml",
            f"version=1\n[server]\nlisten='127.0.0.1:{host_port}'\nworker_threads=2\nmax_inflight=16\ndrain_timeout_seconds=3\n"
            f"[database]\nurl_file={json.dumps(str(runtime_url))}\nmax_connections=4\n[console]\nenabled=true\n"
            f"[model]\nconfig_file={json.dumps(str(configured))}\nlocal_http=true\nrouting_authority='file'\n")
        args = argparse.Namespace(repo=repo, release_branch="reviewed", branch="ai/channel-two", config="models.toml",
            local_http=True, plan=write(root / "intent.json", json.dumps(intent)), instance=f"http://127.0.0.1:{host_port}",
            allow_insecure_loopback=True, credential_file=str(session), token_stdin=False, core=str(opts.core), installed=configured)
        opener = build_opener(ProxyHandler({}), change.admin["NoRedirect"]())

        def readback():
            req = Request(args.instance + change.CONFIG, headers={"Authorization": "Bearer " + user["secret"], "Cache-Control": "no-cache"})
            with opener.open(req, timeout=2) as response:
                return json.load(response)["data"]

        def stop(proc):
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=15)

        def start(label):
            log = root / (label + ".log")
            with log.open("w") as stream:
                proc = subprocess.Popen([opts.core, "--config", host, "serve"], stdout=stream, stderr=stream)
            cleanup.callback(stop, proc)
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline:
                if proc.poll() is not None:
                    raise RuntimeError(f"Core failed to start ({label}): {log.read_text()[-800:]}")
                try:
                    return proc, readback()
                except (OSError, ValueError, KeyError):
                    time.sleep(0.15)
            raise RuntimeError("Authenticated active-configuration readback did not become available")

        first, base = start("initial")
        assert base["authority"]["mode"] == "file" and base["authority"]["source"] == "model.config_file"
        assert base["version"] == {"routing": 12, "pricing": 4}
        proposal = change.propose(args)
        args.commit = proposal["commit"]
        assert change.status(args)["state"] == "awaiting_review"
        run(git + ["merge", "--ff-only", "-q", args.commit])
        assert change.status(args)["state"] == "reviewed"
        published = change.install(args)
        pending = change.status(args)
        assert published["state"] == pending["state"] == "published" and pending["effective"] is False
        assert readback()["version"] == base["version"]
        stop(first)
        second, actual = start("restarted")
        final = change.status(args)
        assert second.pid != first.pid and final["state"] == "effective" and final["effective"] is True
        assert actual["prices"] == base["prices"] and actual["version"] == {"routing": 13, "pricing": 4}
        rows = actual["routing"]["routes"]
        assert len(rows) == proposal["priced_bindings"] == 14 and len(proposal["draft_bindings"]) == 4
        for row in rows:
            assert [t["upstream_id"] for t in row["targets"]] == [1, 2]
        print(json.dumps({"state": final["state"], "postgres": pg_version, "proposal_commit": args.commit,
            "priced_bindings": len(rows), "draft_bindings": len(proposal["draft_bindings"]),
            "instances": [first.pid, second.pid], "before": base["authority"], "after": actual["authority"],
            "expected": final["expected"], "after_install_before_restart": pending["state"]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
