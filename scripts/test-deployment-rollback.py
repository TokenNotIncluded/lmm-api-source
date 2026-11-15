#!/usr/bin/env python3
"""Run the real deployment playbook against disposable files and an HTTP server."""
import functools
import http.server
import importlib.util
import json
import os
from pathlib import Path
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
        if "rejected-candidate" in Path(self.directory, "index.html").read_text():
            self.send_error(503)
        else:
            super().do_GET()


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
            for number, content, should_pass in [(1, "accepted-candidate", True), (2, "rejected-candidate", False)]:
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
                result = subprocess.run([os.environ.get("ANSIBLE_PLAYBOOK_BIN", "ansible-playbook"),
                                         "-i", str(inventory), str(ROOT / "deployment/automation/deploy.yml")],
                                        capture_output=True, text=True)
                if (result.returncode == 0) != should_pass:
                    raise AssertionError(result.stdout + result.stderr)
                if not should_pass:
                    assert "preceding component links have been restored" in result.stdout, result.stdout
                assert (root / "web/dist/index.html").read_text() == "<!doctype html>accepted-candidate"
                assert len(list((root / "slots").iterdir())) == 1, "historical slot was not cleaned"
                assert not list(root.glob(".incoming-*.tar.gz")), "uploaded archive was not cleaned"
                print("PASS:", "successful deployment and cleanup" if should_pass else "HTTP failure, rollback and cleanup")
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    main()
