"""Owned static Nginx sibling for the existing whole-budget acceptance gate.

The caller supplies a reviewed Web file map and its immutable evidence bindings.
This helper never builds Web, reads installed Nginx configuration, or changes the
existing Core/Extensions product group.
"""
from __future__ import annotations

import hashlib
import errno
import json
import os
from pathlib import Path, PurePosixPath
import re
import socket
import stat
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

import resource_group as rg


def fixed_file(path: Path, expected: str) -> Path:
    if (not path.is_absolute() or path.resolve() != path
            or not isinstance(expected, str)
            or not re.fullmatch(r"[0-9a-f]{64}", expected)):
        raise rg.ResourceError("explicit regular file and SHA256 required")
    info = path.lstat()
    if (not stat.S_ISREG(info.st_mode) or info.st_mode & 0o022
            or rg.sha(path) != expected):
        raise rg.ResourceError("fixed Web input changed or is unsafe")
    return path


def web_files(directory: Path, expected: dict[str, str]) -> None:
    if (not directory.is_absolute() or directory.resolve() != directory
            or not directory.is_dir() or "index.html" not in expected):
        raise rg.ResourceError("fixed Web directory and index required")
    for name, digest in expected.items():
        relative = PurePosixPath(name)
        if (relative.is_absolute() or str(relative) != name
                or any(part in {".", ".."} for part in relative.parts)):
            raise rg.ResourceError("unsafe Web manifest path")
        fixed_file(directory / name, digest)
    actual = set()
    for path in directory.rglob("*"):
        if path.is_symlink():
            raise rg.ResourceError("Web input symlinks are forbidden")
        if path.is_file():
            actual.add(path.relative_to(directory).as_posix())
        elif not path.is_dir():
            raise rg.ResourceError("Web tree contains a non-regular entry")
    if actual != set(expected):
        raise rg.ResourceError("Web tree differs from its complete manifest")


def load_input(path: Path, expected: str) -> dict:
    """Bind caller-reviewed build evidence; this does not qualify a Web build.

    The complete artifact map is a plain relative-path -> SHA256 JSON object.
    Original build/source receipts keep their own provenance and are not relabeled
    as this runner's source commit. Their review must precede this input's sealing.
    """
    data = json.loads(fixed_file(path, expected).read_text())
    required = {"format", "nginx", "nginx_sha256", "directory", "manifest", "receipt",
                "source_inputs", "mime_types", "mime_sha256", "probes"}
    if (type(data) is not dict or set(data) != required
            or data["format"] != "lmm-whole-budget-static-web-v1"):
        raise rg.ResourceError("unsupported static Web input format or fields")
    bindings = {"input": (path, expected)}
    for name in ("manifest", "receipt", "source_inputs"):
        item = data[name]
        if type(item) is not dict or set(item) != {"path", "sha256"}:
            raise rg.ResourceError("fixed Web evidence path and digest required")
        evidence = fixed_file(Path(item["path"]), item["sha256"])
        bindings[name] = (evidence, item["sha256"])
    files = json.loads(bindings["manifest"][0].read_text())
    if type(files) is not dict or any(type(key) is not str for key in files):
        raise rg.ResourceError("complete Web artifact SHA256 map required")
    probes = data["probes"]
    if (type(probes) is not list or any(type(p) is not dict or set(p) != {"uri", "file"}
                                      or any(type(p[key]) is not str for key in ("uri", "file"))
                                      for p in probes)
            or not any(p == {"uri": "/", "file": "index.html"} for p in probes)
            or not any(p["file"] == "index.html" and p["uri"] != "/" for p in probes)
            or not all(any(p["file"].endswith(suffix) for p in probes) for suffix in (".js", ".css"))):
        raise rg.ResourceError("home, reviewed SPA route, JavaScript and CSS probes required")
    result = {"nginx": Path(data["nginx"]), "nginx_sha256": data["nginx_sha256"],
              "directory": Path(data["directory"]), "files": files,
              "mime_types": Path(data["mime_types"]), "mime_sha256": data["mime_sha256"],
              "bindings": bindings, "probes": probes}
    fixed_file(result["nginx"], result["nginx_sha256"])
    fixed_file(result["mime_types"], result["mime_sha256"])
    web_files(result["directory"], files)
    return result


def nginx_string(value: str) -> str:
    if "$" in value or any(ord(char) < 32 for char in value):
        raise rg.ResourceError("unsafe Nginx configuration string")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


class StaticWeb:
    def __init__(self, root: Path, whole_budget, *, nginx: Path,
                 nginx_sha256: str, directory: Path, files: dict[str, str],
                 mime_types: Path, mime_sha256: str,
                 bindings: dict[str, tuple[Path, str]],
                 probes: list[dict[str, str]]):
        rg.private_root(root)
        if not {"manifest", "receipt"}.issubset(bindings) or os.geteuid() == 0:
            raise rg.ResourceError("reviewed Web manifest/receipt and non-root controller required")
        self.root, self.whole = root, whole_budget
        self.owner = rg.identity(os.getpid())
        self.nginx, self.nginx_sha256 = nginx, nginx_sha256
        self.directory, self.files = directory, dict(files)
        self.mime_types, self.mime_sha256 = mime_types, mime_sha256
        self.bindings, self.probes = dict(bindings), list(probes)
        self.unit = "lmmjoint" + self.whole.nonce + "-web.service"
        self.description = "Owned static Web acceptance " + self.whole.nonce
        self.group = rg.verify_whole_lease(self.whole.lease()) / self.unit
        self.started = self.ready = self.closed = False
        self.master = None
        self.binary_identity = None
        self.port = None
        self.check_inputs()
        for probe in self.probes:
            uri = urllib.parse.urlsplit(probe["uri"])
            if (not probe["uri"].startswith("/") or uri.scheme or uri.netloc
                    or uri.fragment or probe["file"] not in self.files):
                raise rg.ResourceError("probe must bind a local URI to a fixed Web file")
        if not self.probes:
            raise rg.ResourceError("fixed static Web probes required")

    def check_inputs(self):
        fixed_file(self.nginx, self.nginx_sha256)
        fixed_file(self.mime_types, self.mime_sha256)
        for path, digest in self.bindings.values():
            fixed_file(path, digest)
        web_files(self.directory, self.files)

    def _observed(self):
        observed = rg.properties(self.unit, manager="system")
        if observed.get("LoadState") == "not-found":
            return observed
        group = observed.get("ControlGroup")
        expected = "/" + self.whole.unit + "/" + self.unit
        # systemd clears ControlGroup after an inactive unit loses its last PID.
        # A missing path is also possible before Type=exec starts the first PID.
        empty = (group == "" and observed.get("MainPID", "0") == "0"
                 and not self.group.exists())
        if (observed.get("Transient") != "yes"
                or observed.get("Description") != self.description
                or observed.get("Slice") != self.whole.unit
                or (group != expected and not empty)):
            raise rg.ResourceError("refuse foreign or misplaced Web unit")
        return observed

    def _config(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            self.port = listener.getsockname()[1]
        quote = nginx_string
        temp_paths = {}
        for kind in ("client_body", "proxy", "fastcgi", "uwsgi", "scgi"):
            path = self.root / (kind + "_temp")
            path.mkdir(mode=0o700)
            temp_paths[kind] = path
        with (self.root / "nginx-error.log").open("x"):
            os.chmod(self.root / "nginx-error.log", 0o600)
        config = (
            "worker_processes 1;\n"
            f"pid {quote(str(self.root / 'nginx.pid'))};\n"
            f"lock_file {quote(str(self.root / 'nginx.lock'))};\n"
            f"error_log {quote(str(self.root / 'nginx-error.log'))};\n"
            "events { worker_connections 1024; }\nhttp {\n"
            f"  include {quote(str(self.mime_types))};\n"
            + "".join(f"  {kind}_temp_path {quote(str(path))};\n"
                      for kind, path in temp_paths.items())
            +
            "  default_type application/octet-stream;\n"
            "  access_log off;\n  server {\n"
            f"    listen 127.0.0.1:{self.port};\n"
            "    server_name localhost;\n    location / {\n"
            f"      root {quote(str(self.directory))};\n"
            "      index index.html;\n      try_files $uri $uri/ /index.html;\n"
            '      add_header Cache-Control "no-cache" always;\n'
            "    }\n  }\n}\n"
        )
        path = self.root / "nginx.conf"
        with path.open("x") as target:
            os.chmod(path, 0o600)
            target.write(config)
        return path

    def start(self):
        if self.started or self.closed:
            raise rg.ResourceError("Web creation is one-shot")
        rg.verify_whole_lease(self.whole.lease())
        self.check_inputs()
        config = self._config()
        error_log = str(self.root / "nginx-error.log")
        argv = [str(self.nginx), "-e", error_log, "-p", str(self.root) + "/", "-c", str(config),
                "-g", "daemon off;"]
        checked = subprocess.run(
            [str(self.nginx), "-t", "-e", error_log,
             "-p", str(self.root) + "/", "-c", str(config)],
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"},
            text=True, capture_output=True, timeout=15,
        )
        rg.record(self.root, "web-nginx-test.json", {
            "exit_code": checked.returncode, "stdout": checked.stdout,
            "stderr": checked.stderr, "configuration_sha256": rg.sha(config),
        })
        if checked.returncode:
            raise rg.ResourceError("private Nginx configuration check failed")
        rg.record(self.root, "web-allocation.json", {
            "unit": self.unit, "owner": self.owner, "whole_budget": self.whole.lease(),
            "binary_sha256": self.nginx_sha256, "configuration_sha256": rg.sha(config),
            "mime_sha256": self.mime_sha256, "web_files": self.files,
            "evidence": {name: {"path": str(path), "sha256": digest}
                         for name, (path, digest) in self.bindings.items()},
            "probes": self.probes, "port": self.port,
        })
        self.started = True
        log = self.root / "nginx-service.log"
        with log.open("x"):
            os.chmod(log, 0o600)
        settings = [
            ("Description", "s", self.description), ("Slice", "s", self.whole.unit),
            ("Type", "s", "exec"), ("User", "s", str(os.geteuid())),
            ("Group", "s", str(os.getegid())), ("NoNewPrivileges", "b", "true"),
            ("CPUAccounting", "b", "true"), ("MemoryAccounting", "b", "true"),
            ("WorkingDirectory", "s", str(self.root)),
            ("Environment", "as", "2", "PATH=/usr/bin:/bin", "LANG=C.UTF-8"),
            ("StandardOutput", "s", "append:" + str(log)),
            ("StandardError", "s", "append:" + str(log)),
            ("RuntimeMaxUSec", "t", "900000000"), ("TimeoutStopUSec", "t", "10000000"),
            ("KillMode", "s", "control-group"), ("Restart", "s", "no"),
            ("ExecStart", "a(sasb)", "1", str(self.nginx), str(len(argv)), *argv, "false"),
        ]
        command = [*rg.manager_command("/usr/bin/busctl", "system"), "call",
                   "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
                   "org.freedesktop.systemd1.Manager", "StartTransientUnit",
                   "ssa(sv)a(sa(sv))", self.unit, "fail", str(len(settings))]
        for setting in settings:
            command.extend(setting)
        command.append("0")
        with self.nginx.open("rb") as pinned:
            info = os.fstat(pinned.fileno())
            if hashlib.file_digest(pinned, "sha256").hexdigest() != self.nginx_sha256:
                raise rg.ResourceError("opened Nginx binary differs from fixed input")
            self.binary_identity = (info.st_dev, info.st_ino)
            result = subprocess.run(command, env=rg.bus_environment("system"),
                                    text=True, capture_output=True, timeout=15)
            rg.record(self.root, "web-start.json", {
                "exit_code": result.returncode, "stdout": result.stdout,
                "stderr": result.stderr,
            })
            if result.returncode:
                raise rg.ResourceError("owned Web start failed; inspect private receipt")
            deadline = time.monotonic() + 15
            while True:
                observed = self._observed()
                pid = int(observed.get("MainPID", "0"))
                if observed.get("ActiveState") == "active" and pid > 0:
                    try:
                        current = rg.identity(pid)
                        if self.master is not None and self.master != current:
                            raise rg.ResourceError("Web master changed during startup")
                        self.master = current
                        self._processes(pid)
                    except OSError as error:
                        # Match the existing product launcher's bounded /proc
                        # retry; neither denial nor disappearance proves exec.
                        if error.errno not in {errno.EACCES, errno.ENOENT}:
                            raise
                    else:
                        self.ready = True
                        try:
                            self.probe("web-ready-probes.json")
                            break
                        except urllib.error.URLError as error:
                            if not isinstance(error.reason, ConnectionRefusedError):
                                raise
                if time.monotonic() >= deadline or observed.get("ActiveState") == "failed":
                    raise rg.ResourceError("owned Web did not become ready")
                time.sleep(0.025)
        rg.record(self.root, "web-ready.json", {
            "unit": self.unit, "owner": self.owner, "master": self.master,
            "cgroup": str(self.group), "port": self.port,
        })
        return self

    def _processes(self, master_pid):
        pids = [int(value) for value in (self.group / "cgroup.procs").read_text().split()]
        if master_pid not in pids:
            raise rg.ResourceError("Nginx master is outside its owned cgroup")
        observed = []
        for pid in pids:
            proc = Path("/proc") / str(pid)
            info = (proc / "exe").stat()
            fields = dict(line.split(":", 1) for line in (proc / "status").read_text().splitlines()
                          if ":" in line)
            if (self.binary_identity != (info.st_dev, info.st_ino)
                    or set(fields["Uid"].split()) != {str(os.geteuid())}
                    or (proc / "cgroup").read_text().strip()
                    != "0::/" + self.whole.unit + "/" + self.unit):
                raise rg.ResourceError("foreign process in owned Web cgroup")
            observed.append(rg.identity(pid))
        return observed

    def sample(self):
        rg.verify_whole_lease(self.whole.lease())
        observed = self._observed()
        if (not self.ready or self.closed or observed.get("ActiveState") != "active"
                or rg.identity(int(observed.get("MainPID", "0"))) != self.master):
            raise rg.ResourceError("original Web master is no longer active")
        return {"cgroup": str(self.group), "master": self.master,
                "processes": self._processes(self.master["pid"]), **rg.metrics(self.group)}

    def probe(self, receipt_name):
        self.check_inputs()
        self.sample()
        results = []
        for probe in self.probes:
            target = self.directory / probe["file"]
            request = urllib.request.Request(
                f"http://127.0.0.1:{self.port}" + probe["uri"], method="GET")
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect)
            with opener.open(request, timeout=5) as response:
                body = response.read(target.stat().st_size + 1)
                if (response.status != 200 or len(body) != target.stat().st_size
                        or hashlib.sha256(body).hexdigest() != self.files[probe["file"]]
                        or response.headers.get("Cache-Control") != "no-cache"):
                    raise rg.ResourceError("static Web response differs from fixed input")
            results.append({"uri": probe["uri"], "file": probe["file"],
                            "sha256": self.files[probe["file"]], "status": 200})
        rg.record(self.root, receipt_name, {"unit": self.unit, "results": results})

    def close(self):
        if self.closed:
            return
        if rg.identity(os.getpid()) != self.owner:
            raise rg.ResourceError("Web cleanup is limited to its controller")
        if not self.started:
            self.closed = True
            rg.record(self.root, "web-closed.json", {
                "unit": self.unit, "owner": self.owner, "creation_attempted": False,
            })
            return
        observed = self._observed()
        if observed.get("LoadState") != "not-found":
            pid = int(observed.get("MainPID", "0"))
            if pid > 0:
                if self.master and rg.identity(pid) != self.master:
                    raise rg.ResourceError("refuse cleanup of a replaced Web master")
                self._processes(pid)
            rg.control("stop", self.unit, manager="system")
        observed = self._observed()
        if ((observed.get("LoadState") != "not-found"
             and (observed.get("ActiveState") not in {"inactive", "failed"}
                  or observed.get("MainPID", "0") != "0"))
                or self.group.exists()):
            raise rg.ResourceError("owned Web removal unconfirmed")
        if self.master:
            try:
                if rg.identity(self.master["pid"]) == self.master:
                    raise rg.ResourceError("original Web master remains")
            except FileNotFoundError:
                pass
        self.closed = True
        rg.record(self.root, "web-closed.json", {
            "unit": self.unit, "owner": self.owner, "processes_and_cgroups_removed": True,
        })
