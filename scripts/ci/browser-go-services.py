"""Start the production Extensions HTTP host for the isolated Chromium workflow."""
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request


def private_file(path, value):
    with path.open("x") as handle:
        handle.write(value)
    return path


def provision(component, directory, binary, port, admin_password, clean_env):
    # Both the installer login and the runtime login have no cluster privileges.
    db = f"lmm_{component}_{secrets.token_hex(5)}"
    owner, runtime = db + "_owner", db + "_runtime"
    owner_pass, runtime_pass = secrets.token_hex(24), secrets.token_hex(24)
    sql = f"""
CREATE ROLE {owner} LOGIN PASSWORD '{owner_pass}';
CREATE ROLE {runtime} LOGIN PASSWORD '{runtime_pass}';
CREATE DATABASE {db} OWNER {owner};
REVOKE ALL ON DATABASE {db} FROM PUBLIC;
"""
    admin = dict(clean_env, PGHOST="127.0.0.1", PGPORT=str(port), PGUSER="postgres",
                 PGPASSWORD=admin_password, PGDATABASE="postgres")
    result = subprocess.run(["psql", "-Xq", "-v", "ON_ERROR_STOP=1"], input=sql, text=True,
                            env=admin, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if result.returncode:
        raise RuntimeError(f"{component} disposable database provisioning failed")
    def dsn(role, password):
        return f"postgresql://{role}:{password}@127.0.0.1:{port}/{db}?sslmode=disable"

    installer = private_file(directory / (component + ".owner"), dsn(owner, owner_pass))
    try:
        output = subprocess.check_output([
            str(binary), "install-db", "--component", component, "--owner-url-file", str(installer),
            "--database", db, "--owner-role", owner, "--runtime-role", runtime,
        ], env=clean_env)
    finally:
        installer.unlink()
    receipt = json.loads(output)
    if receipt["component"] != component or receipt["database"] != db or not receipt["schema_sha256"]:
        raise RuntimeError(f"{component} installer receipt did not identify its schema")
    return db, private_file(directory / (component + ".runtime"), dsn(runtime, runtime_pass))


def start_host(listener, binary, config, directory, clean_env):
    pid = os.fork()
    if pid == 0:
        try:
            os.setsid()
            # Use the production systemd socket contract, retaining the kernel-selected port.
            os.dup2(listener.fileno(), 3, inheritable=True)
            os.set_inheritable(3, True)
            with open(os.devnull, "rb") as src, (directory / "service.log").open("xb") as log:
                os.dup2(src.fileno(), 0)
                os.dup2(log.fileno(), 1)
                os.dup2(log.fileno(), 2)
            env = dict(clean_env, LISTEN_PID=str(os.getpid()), LISTEN_FDS="1",
                       LISTEN_FDNAMES="lmm-extensions-http")
            os.execve(binary, [str(binary), "--config", str(config)], env)
        finally:
            os._exit(127)
    private_file(directory / "pid", str(pid))
    return pid


def check_routes(url, pid):
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    for _ in range(80):
        if os.waitpid(pid, os.WNOHANG)[0]:
            raise RuntimeError("Extensions exited before its HTTP routes became ready")
        try:
            with http.open(url + "/api/site/navigation", timeout=1) as response:
                navigation = json.load(response)
            break
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"Extensions navigation returned HTTP {error.code}") from None
        except urllib.error.URLError:
            time.sleep(0.15)
    else:
        raise RuntimeError("Extensions navigation did not become ready")
    if navigation.get("success") is not True or not isinstance(navigation["data"]["revision"], int):
        raise RuntimeError("Extensions did not return the persisted community navigation record")
    try:
        http.open(url + "/api/release-notes/latest", timeout=2).close()
    except urllib.error.HTTPError as error:
        if error.code == 401 and json.load(error).get("success") is False:
            return
    raise RuntimeError("Extensions must reject anonymous release-note reads")


def start(directory, binary):
    os.umask(0o077)
    directory.mkdir(mode=0o700)
    clean_env = {key: os.environ[key] for key in ("PATH", "HOME", "RUNNER_TRACKING_ID") if key in os.environ}
    port = int(os.environ["BROWSER_EXTENSIONS_PG_PORT"])
    if not 1024 <= port <= 65535:
        raise ValueError("expected the dedicated disposable PostgreSQL service port")
    components = {name: provision(name, directory, binary, port,
                                  os.environ["BROWSER_EXTENSIONS_PG_PASSWORD"], clean_env)
                  for name in ("community", "operations")}
    credentials = {name: private_file(directory / (name + ".token"), secrets.token_hex(32))
                   for name in ("host", "identity")}
    rpc_socket = directory / "identity.sock"
    if len(os.fsencode(rpc_socket)) > 100:
        raise ValueError("Core identity socket path exceeds its native bound")
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        addr = f"127.0.0.1:{listener.getsockname()[1]}"
        url = "http://" + addr
        configuration = [
            "version = 1", "[host]", f'listen = "{addr}"', f'public_origin = "{url}"',
            "local_http = true", "web_compat = true", 'modules = ["community", "operations"]',
            "token_file = " + json.dumps(str(credentials["host"])), "[core]",
            "socket = " + json.dumps(str(rpc_socket)),
            "identity_token_file = " + json.dumps(str(credentials["identity"])),
        ]
        for name, (db, runtime_url) in components.items():
            configuration.extend([f"[{name}]", "database_name = " + json.dumps(db),
                                  "database_url_file = " + json.dumps(str(runtime_url))])
        config = private_file(directory / "extensions.toml", "\n".join(configuration) + "\n")
        pid = start_host(listener, binary, config, directory, clean_env)
    check_routes(url, pid)
    with open(os.environ["GITHUB_ENV"], "a") as out:
        print("BROWSER_EXTENSIONS_URL=" + url, file=out)
        print("BROWSER_CORE_RPC_SOCKET=" + str(rpc_socket), file=out)
        print("BROWSER_CORE_RPC_TOKEN_FILE=" + str(credentials["identity"]), file=out)
    print("Extensions public navigation read returned its PG revision and anonymous release-notes returned 401.")


def stop(directory, binary):
    pid_file = directory / "pid"
    if pid_file.exists():
        pid = int(pid_file.read_text())
        try:
            if Path(f"/proc/{pid}/exe").resolve(strict=True) == binary:
                os.kill(pid, signal.SIGTERM)
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("This fixture requires a GitHub-hosted workflow job.")
    temp = Path(os.environ["RUNNER_TEMP"]).resolve()
    directory, binary = temp / "browser-extensions", temp / "browser-extensions-bin"
    if sys.argv[1:] == ["start"]:
        start(directory, binary)
    elif sys.argv[1:] == ["stop"]:
        stop(directory, binary)
    else:
        raise SystemExit("expected start or stop")
