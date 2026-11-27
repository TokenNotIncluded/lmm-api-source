#!/usr/bin/env python3
"""Run the real deployment playbook against disposable files and an HTTP server."""
import functools
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("deployment_plan", ROOT / "scripts/deployment-plan.py")
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)
spec = importlib.util.spec_from_file_location("distribution", ROOT / "scripts/build-distribution.py")
distribution = importlib.util.module_from_spec(spec)
spec.loader.exec_module(distribution)


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        if self.path.startswith("/backend/"):
            service = self.path.rsplit("/", 1)[-1]
            audit = getattr(self.server, "rollback_audit", None)
            if audit is not None:
                with audit.open("a") as sink:
                    sink.write(f"ready {service}\n")
            content_type = "text/html" if service == "html" else "application/json"
            body = (b"<!doctype html>SPA fallback" if service == "html" else
                    json.dumps({"status": "ready", "service": service}).encode())
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.end_headers()
            self.wfile.write(body)
            return
        if "rejected-candidate" in Path(self.directory, "index.html").read_text():
            self.send_error(503)
        else:
            super().do_GET()


def verify_backend_readiness(work, server):
    # Exercise the real verification tasks against this test process. The fake
    # systemctl only reports its PID and never touches the host service manager.
    fake_bin = work / "bin"
    fake_bin.mkdir()
    systemctl = fake_bin / "systemctl"
    systemctl.write_text(f"#!/bin/sh\nprintf '%s\\n' {os.getpid()}\n")
    systemctl.chmod(0o755)
    digest = hashlib.sha256(Path("/proc/self/exe").read_bytes()).hexdigest()
    playbook = work / "backend-readiness.yml"
    for service, accepted in [("lmm-core", True), ("lmm-extensions", False), ("html", False)]:
        component = {
            "name": "core",
            "service": "synthetic-core.service",
            "health_url": f"http://127.0.0.1:{server.server_port}/backend/{service}",
            "files": {"bin/lmm-core": {"sha256": digest}},
        }
        playbook.write_text(json.dumps([{
            "hosts": "localhost",
            "connection": "local",
            "gather_facts": False,
            "environment": {"PATH": str(fake_bin) + os.pathsep + os.environ["PATH"]},
            "vars": {"component": component},
            "tasks": [{
                "ansible.builtin.include_tasks": str(ROOT / "deployment/automation/verify.yml"),
            }],
        }]))
        result = subprocess.run([os.environ.get("ANSIBLE_PLAYBOOK_BIN", "ansible-playbook"),
                                 "-i", "localhost,", str(playbook)],
                                capture_output=True, text=True, check=False)
        if (result.returncode == 0) != accepted:
            raise AssertionError(result.stdout + result.stderr)
        if not accepted:
            assert "did not return readiness for the selected service" in result.stdout, result.stdout
    print("PASS: recognized readiness, wrong service rejected, HTML fallback rejected")

    root = work / "backend"
    slot = root / "slots/core-a"
    binary = slot / "bin/lmm-core"
    binary.parent.mkdir(parents=True)
    shutil.copy2("/proc/self/exe", binary)
    files = {"bin/lmm-core": {"sha256": digest, "size": binary.stat().st_size,
                               "mode": stat.S_IMODE(binary.stat().st_mode)}}
    (slot / "MANIFEST.json").write_text(json.dumps({"revision": "a" * 40, "files": files}))
    tools = root / ".tools/scripts"
    tools.mkdir(parents=True)
    shutil.copyfile(ROOT / "scripts/build-distribution.py", tools / "build-distribution.py")
    for service, queued in [("lmm-core", False), ("lmm-extensions", True)]:
        component["previous"] = str(slot)
        component["health_url"] = f"http://127.0.0.1:{server.server_port}/backend/{service}"
        component["files"] = files
        plan = [{
            "hosts": "localhost",
            "connection": "local",
            "gather_facts": False,
            "environment": {"PATH": str(fake_bin) + os.pathsep + os.environ["PATH"]},
            "vars": {"component": component, "deploy_root": str(root), "deploy_revision": "a" * 40,
                     "ansible_python_interpreter": sys.executable, "deploy_states": []},
            "tasks": [
                {"ansible.builtin.include_tasks": str(ROOT / "deployment/automation/plan-activation.yml")},
                {"ansible.builtin.assert": {"that": f"(deploy_states | length) == {int(queued)}"}},
            ],
        }]
        playbook.write_text(json.dumps(plan))
        result = subprocess.run([os.environ.get("ANSIBLE_PLAYBOOK_BIN", "ansible-playbook"),
                                 "-i", "localhost,", str(playbook)],
                                capture_output=True, text=True)
        assert result.returncode == 0, result.stdout + result.stderr
    print("PASS: identical running Core skips restart; mismatching readiness queues activation")



def verify_backend_rollback(work, server):
    fake_bin = work / "rollback-bin"
    fake_bin.mkdir()
    audit = work / "rollback-actions"
    audit.touch()
    systemctl = fake_bin / "systemctl"
    systemctl.write_text(r"""#!/bin/sh
set -eu
case "$1" in
  show)
    printf 'Id=%s\nLoadState=loaded\nActiveState=failed\nSubState=failed\nResult=start-limit-hit\n' "$2"
    ;;
  reset-failed)
    printf 'clear %s\n' "$2" >> "$LMM_ROLLBACK_AUDIT"
    ;;
  start|restart)
    grep -Fxq "clear $2" "$LMM_ROLLBACK_AUDIT"
    if [ "$2" = lmm-extensions.service ]; then
      grep -Fxq 'ready lmm-core' "$LMM_ROLLBACK_AUDIT"
    fi
    printf 'restart %s\n' "$2" >> "$LMM_ROLLBACK_AUDIT"
    ;;
  *) exit 1 ;;
esac
""")
    systemctl.chmod(0o755)
    components = [{
        "name": name,
        "service": f"lmm-{name}.service",
        "health_url": f"http://127.0.0.1:{server.server_port}/backend/lmm-{name}",
    } for name in ("core", "extensions", "web")]
    playbook = work / "backend-rollback.yml"
    playbook.write_text(json.dumps([{
        "hosts": "localhost",
        "connection": "local",
        "gather_facts": False,
        "environment": {
            "PATH": str(fake_bin) + os.pathsep + os.environ["PATH"],
            "LMM_ROLLBACK_AUDIT": str(audit),
        },
        "tasks": [{
            "ansible.builtin.include_tasks": str(ROOT / "deployment/automation/rollback.yml"),
            "loop": components,
            "loop_control": {"loop_var": "component"},
        }],
    }]))
    server.rollback_audit = audit
    try:
        result = subprocess.run(
            [os.environ.get("ANSIBLE_PLAYBOOK_BIN", "ansible-playbook"),
             "-i", "localhost,", str(playbook)],
            capture_output=True, text=True,
        )
    finally:
        del server.rollback_audit
    if result.returncode:
        raise AssertionError(result.stdout + result.stderr)
    assert audit.read_text().splitlines() == [
        "clear lmm-core.service",
        "restart lmm-core.service",
        "ready lmm-core",
        "clear lmm-extensions.service",
        "restart lmm-extensions.service",
        "ready lmm-extensions",
        "ready lmm-web",
    ], audit.read_text()
    print("PASS: cleared systemd start limits and verified restored Core before restarting extensions")


def main():
    revision = "a" * 40
    with tempfile.TemporaryDirectory(prefix="lmm-deploy-fixture-") as temp:
        work = Path(temp)
        root = work / "node"
        (root / "slots/web-a/dist").mkdir(parents=True)
        (root / "slots/web-a/dist/index.html").write_text("<!doctype html>previous")
        (root / "web").symlink_to(root / "slots/web-a", target_is_directory=True)
        (root / ".managed").write_text("lmm-deployment-v1\n")
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(root / "web/dist")))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        target = {"name": "fixture", "host": "127.0.0.1", "root": str(root),
                  "min_free_bytes": 1024,
                  "components": {"web": {"health_url": f"http://127.0.0.1:{server.server_port}/index.html"}}}
        try:
            verify_backend_readiness(work, server)
            verify_backend_rollback(work, server)
            cases = [(1, "accepted-candidate", True, True),
                     (2, "rejected-candidate", False, True),
                     (3, "rejected-candidate", False, False)]
            for number, content, should_pass, allow_rollback in cases:
                if not allow_rollback:
                    target["allow_rollback"] = False
                source = work / f"input-{number}"
                source.mkdir()
                (source / "index.html").write_text("<!doctype html>" + content)
                archives = work / f"artifacts-{number}"
                distribution.build(ROOT, "web", "1.2." + str(number), revision, "any", source, archives)
                inventory = work / "inventory.json"
                with patch.dict(os.environ, {"DEPLOY_TARGETS": json.dumps([target]),
                                            "DEPLOY_REVISION": revision,
                                            "SELECT_TARGETS": "all", "SELECT_COMPONENTS": "web"}):
                    deploy.inventory(archives, inventory)
                hosts = json.loads(inventory.read_text())
                hosts["all"]["hosts"]["fixture"].update({"ansible_connection": "local", "ansible_python_interpreter": sys.executable})
                inventory.write_text(json.dumps(hosts))
                preceding_slot = (root / "web").resolve()
                result = subprocess.run([os.environ.get("ANSIBLE_PLAYBOOK_BIN", "ansible-playbook"),
                                         "-i", str(inventory), str(ROOT / "deployment/automation/deploy.yml")],
                                        capture_output=True, text=True)
                if (result.returncode == 0) != should_pass:
                    raise AssertionError(result.stdout + result.stderr)
                if not allow_rollback:
                    assert "automatic rollback is disabled" in result.stdout, result.stdout
                    assert (root / "web/dist/index.html").read_text() == "<!doctype html>rejected-candidate"
                    assert (preceding_slot / "dist/index.html").read_text() == "<!doctype html>accepted-candidate"
                    assert (root / "web").resolve() != preceding_slot
                    assert len(list((root / "slots").iterdir())) == 2, "forward recovery lost a component slot"
                    assert (root / ".incoming-web.tar.gz").is_file(), "forward recovery lost the received archive"
                    print("PASS: HTTP failure retains selected candidate, preceding slot and received archive")
                    continue
                if not should_pass:
                    assert "preceding component links have been restored" in result.stdout, result.stdout
                assert (root / "web/dist/index.html").read_text() == "<!doctype html>accepted-candidate"
                assert len(list((root / "slots").iterdir())) == 1, "historical slot was not cleaned"
                assert not list(root.glob(".incoming-*.tar.gz")), "uploaded archive was not cleaned"
                print("PASS:", "successful deployment and cleanup" if should_pass else "HTTP failure, rollback and cleanup")
                if number == 1:
                    active_slot = (root / "web").resolve()
                    rerun = [os.environ.get("ANSIBLE_PLAYBOOK_BIN", "ansible-playbook"),
                             "-i", str(inventory), str(ROOT / "deployment/automation/deploy.yml")]
                    result = subprocess.run(rerun, capture_output=True, text=True)
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert (root / "web").resolve() == active_slot, "identical release changed slots"
                    assert "Upload the component archive" not in result.stdout, result.stdout
                    # Damage a package file that the HTTP health check never reads.
                    (active_slot / "README.md").write_text("damaged")
                    result = subprocess.run(rerun, capture_output=True, text=True)
                    assert result.returncode == 0, result.stdout + result.stderr
                    assert (root / "web").resolve() != active_slot, "damaged package was skipped"
                    assert "damaged" not in (root / "web/README.md").read_text()
                    print("PASS: unchanged healthy artifact skips upload and activation, damaged artifact is reinstalled")
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    main()
