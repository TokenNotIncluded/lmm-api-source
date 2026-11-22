"""Pure input and failure-cleanup checks; these do not start systemd or Nginx."""
import json
import errno
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import urllib.request

import whole_budget_web as web


class StaticWebBoundaries(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="lmm-web-pure-")
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.root = self.base / "owned"
        self.root.mkdir(mode=0o700)
        self.dist = self.base / "dist"
        self.dist.mkdir()
        (self.dist / "index.html").write_text("<html>fixed</html>")
        self.files = {"index.html": web.rg.sha(self.dist / "index.html")}
        self.nginx = self.base / "nginx"
        self.nginx.write_text("unexecuted synthetic binary")
        self.mime = self.base / "mime.types"
        self.mime.write_text("types { text/html html; }\n")
        self.bindings = {}
        for kind in ("manifest", "receipt"):
            path = self.base / (kind + ".json")
            path.write_text("{}\n")
            self.bindings[kind] = (path, web.rg.sha(path))

        # Exercise individual boundaries without claiming a real WholeBudget
        # lease exists. Runtime lease/proc observations are explicit test doubles.
        self.subject = object.__new__(web.StaticWeb)
        s = self.subject
        s.root, s.directory, s.files = self.root, self.dist, self.files
        s.nginx, s.nginx_sha256 = self.nginx, web.rg.sha(self.nginx)
        s.mime_types, s.mime_sha256 = self.mime, web.rg.sha(self.mime)
        s.bindings = self.bindings
        s.probes = [{"uri": "/", "file": "index.html"}]
        nonce = "a" * 32
        s.whole = SimpleNamespace(nonce=nonce, unit="lmmjoint" + nonce + ".slice",
                                  lease=lambda: {"synthetic": True})
        s.unit = "lmmjoint" + nonce + "-web.service"
        s.description = "Owned static Web acceptance " + nonce
        s.owner = web.rg.identity(os.getpid())
        s.group = self.base / "absent-cgroup"
        s.started = s.ready = s.closed = False
        s.master = s.binary_identity = s.port = None

    def properties(self, **changes):
        observed = {"LoadState": "loaded", "ActiveState": "inactive", "MainPID": "0",
                    "Transient": "yes", "Description": self.subject.description,
                    "Slice": self.subject.whole.unit, "ControlGroup": ""}
        observed.update(changes)
        return observed

    def test_complete_manifest_rejects_changed_extra_and_symlink_files(self):
        web.web_files(self.dist, self.files)
        (self.dist / "index.html").write_text("changed")
        with self.assertRaises(web.rg.ResourceError):
            web.web_files(self.dist, self.files)
        (self.dist / "index.html").write_text("<html>fixed</html>")
        extra = self.dist / "extra.txt"
        extra.write_text("unbound")
        with self.assertRaises(web.rg.ResourceError):
            web.web_files(self.dist, self.files)
        extra.unlink()
        (self.dist / "outside").symlink_to(self.base)
        with self.assertRaises(web.rg.ResourceError):
            web.web_files(self.dist, self.files)

    def sealed_input(self):
        for name in ("entry.js", "style.css"):
            (self.dist / name).write_text("fixed " + name)
            self.files[name] = web.rg.sha(self.dist / name)
        manifest = self.bindings["manifest"][0]
        manifest.write_text(json.dumps(self.files, sort_keys=True))
        source = self.base / "source-inputs.json"
        source.write_text('{"synthetic-source": "not-a-real-build"}\n')
        receipt = self.bindings["receipt"][0]
        data = {
            "format": "lmm-whole-budget-static-web-v1",
            "nginx": str(self.nginx), "nginx_sha256": web.rg.sha(self.nginx),
            "directory": str(self.dist),
            "manifest": {"path": str(manifest), "sha256": web.rg.sha(manifest)},
            "receipt": {"path": str(receipt), "sha256": web.rg.sha(receipt)},
            "source_inputs": {"path": str(source), "sha256": web.rg.sha(source)},
            "mime_types": str(self.mime), "mime_sha256": web.rg.sha(self.mime),
            "probes": [{"uri": "/", "file": "index.html"},
                       {"uri": "/synthetic-route", "file": "index.html"},
                       {"uri": "/entry.js", "file": "entry.js"},
                       {"uri": "/style.css", "file": "style.css"}],
        }
        path = self.base / "input.json"
        path.write_text(json.dumps(data, sort_keys=True))
        return path, data

    def test_reviewed_input_binds_all_evidence_and_refuses_receipt_drift(self):
        path, data = self.sealed_input()
        accepted = web.load_input(path, web.rg.sha(path))
        self.assertEqual(set(accepted["bindings"]), {"input", "manifest", "receipt", "source_inputs"})
        self.assertEqual(accepted["files"], self.files)
        Path(data["receipt"]["path"]).write_text("changed original build receipt")
        with self.assertRaises(web.rg.ResourceError):
            web.load_input(path, web.rg.sha(path))

    def test_input_rejects_incomplete_or_nonstring_probes(self):
        path, data = self.sealed_input()
        for probes in (data["probes"][:2], [{"uri": "/", "file": 3}]):
            changed = {**data, "probes": probes}
            path.write_text(json.dumps(changed, sort_keys=True))
            with self.assertRaises(web.rg.ResourceError):
                web.load_input(path, web.rg.sha(path))

    def test_config_has_only_explicit_private_temp_pid_lock_and_log_paths(self):
        config = self.subject._config().read_text()
        for kind in ("client_body", "proxy", "fastcgi", "uwsgi", "scgi"):
            path = self.root / (kind + "_temp")
            self.assertIn(f'{kind}_temp_path "{path}";', config)
            self.assertEqual(path.stat().st_mode & 0o777, 0o700)
        for name in ("nginx.pid", "nginx.lock", "nginx-error.log"):
            self.assertIn('"' + str(self.root / name) + '"', config)
        self.assertIn("try_files $uri $uri/ /index.html;", config)
        self.assertIn('add_header Cache-Control "no-cache" always;', config)

    def test_bad_nginx_config_never_allocates_a_unit(self):
        failed = subprocess.CompletedProcess([], 1, "", "synthetic invalid config")
        with patch.object(web.rg, "verify_whole_lease"), \
                patch.object(web.subprocess, "run", return_value=failed) as run, \
                patch.object(web.rg, "control") as control:
            with self.assertRaisesRegex(web.rg.ResourceError, "configuration check"):
                self.subject.start()
            self.subject.close()
        self.assertEqual(run.call_count, 1)
        argv = run.call_args.args[0]
        self.assertEqual(argv[:2], [str(self.nginx), "-t"])
        self.assertEqual(argv[argv.index("-e") + 1], str(self.root / "nginx-error.log"))
        self.assertFalse(self.subject.started)
        self.assertFalse((self.root / "web-allocation.json").exists())
        control.assert_not_called()

    def test_uncertain_start_keeps_cleanup_ownership_and_accepts_cleared_cgroup(self):
        passed = subprocess.CompletedProcess([], 0, "", "synthetic config check")
        with patch.object(web.rg, "verify_whole_lease"), \
                patch.object(web.subprocess, "run", side_effect=[
                    passed, subprocess.TimeoutExpired("synthetic bus", 15)]):
            with self.assertRaises(subprocess.TimeoutExpired):
                self.subject.start()
        self.assertTrue(self.subject.started)
        self.assertTrue((self.root / "web-allocation.json").is_file())
        with patch.object(web.rg, "properties", return_value=self.properties()), \
                patch.object(web.rg, "control") as control:
            self.subject.close()
        control.assert_called_once_with("stop", self.subject.unit, manager="system")
        self.assertTrue(self.subject.closed)

    def test_start_retries_transient_proc_denial_under_same_master(self):
        passed = subprocess.CompletedProcess([], 0, "", "synthetic")
        observed = self.properties(
            ActiveState="active", MainPID=str(os.getpid()),
            ControlGroup="/" + self.subject.whole.unit + "/" + self.subject.unit)
        with patch.object(web.rg, "verify_whole_lease"), \
                patch.object(web.rg, "properties", return_value=observed), \
                patch.object(web.subprocess, "run", return_value=passed), \
                patch.object(web.time, "sleep") as sleep, \
                patch.object(self.subject, "_processes", side_effect=[
                    PermissionError(errno.EACCES, "synthetic exec transition"), []]) as processes, \
                patch.object(self.subject, "probe") as probe:
            self.subject.start()
        self.assertEqual(processes.call_count, 2)
        sleep.assert_called_once_with(0.025)
        probe.assert_called_once_with("web-ready-probes.json")
        self.assertEqual(self.subject.master, self.subject.owner)

    def test_start_refuses_replacement_after_first_master_witness(self):
        passed = subprocess.CompletedProcess([], 0, "", "synthetic")
        observed = self.properties(
            ActiveState="active", MainPID=str(os.getpid()),
            ControlGroup="/" + self.subject.whole.unit + "/" + self.subject.unit)
        replacement = {**self.subject.owner, "start_ticks": self.subject.owner["start_ticks"] + 1}
        with patch.object(web.rg, "verify_whole_lease"), \
                patch.object(web.rg, "properties", return_value=observed), \
                patch.object(web.rg, "identity", side_effect=[self.subject.owner, replacement]), \
                patch.object(web.subprocess, "run", return_value=passed), \
                patch.object(web.time, "sleep"), \
                patch.object(self.subject, "_processes", side_effect=PermissionError(errno.EACCES, "transition")), \
                patch.object(self.subject, "probe") as probe:
            with self.assertRaisesRegex(web.rg.ResourceError, "changed during startup"):
                self.subject.start()
        probe.assert_not_called()

    def test_cleanup_refuses_foreign_unit_or_replaced_master(self):
        self.subject.started = True
        with patch.object(web.rg, "properties", return_value=self.properties(Description="foreign")), \
                patch.object(web.rg, "control") as control:
            with self.assertRaisesRegex(web.rg.ResourceError, "foreign"):
                self.subject.close()
            control.assert_not_called()
        self.subject.master = {**self.subject.owner, "start_ticks": -1}
        observed = self.properties(
            ActiveState="active", MainPID=str(os.getpid()),
            ControlGroup="/" + self.subject.whole.unit + "/" + self.subject.unit)
        with patch.object(web.rg, "properties", return_value=observed), \
                patch.object(web.rg, "control") as control:
            with self.assertRaisesRegex(web.rg.ResourceError, "replaced"):
                self.subject.close()
            control.assert_not_called()

    def test_loopback_probe_disables_inherited_proxy_and_checks_fixed_body(self):
        response = Mock(status=200, headers={"Cache-Control": "no-cache"})
        response.read.return_value = (self.dist / "index.html").read_bytes()
        response.__enter__ = Mock(return_value=response)
        response.__exit__ = Mock(return_value=False)
        opener = Mock()
        opener.open.return_value = response
        self.subject.port = 43210
        with patch.dict(os.environ, {"HTTP_PROXY": "http://unreachable.invalid:1"}), \
                patch.object(self.subject, "sample"), \
                patch.object(web.urllib.request, "build_opener", return_value=opener) as build:
            self.subject.probe("probe.json")
        handler = build.call_args.args[0]
        self.assertIsInstance(handler, urllib.request.ProxyHandler)
        self.assertEqual(handler.proxies, {})
        self.assertIs(build.call_args.args[1], web._NoRedirect)
        request = opener.open.call_args.args[0]
        self.assertEqual(request.full_url, "http://127.0.0.1:43210/")
        self.assertEqual(json.loads((self.root / "probe.json").read_text())["results"][0]["sha256"],
                         self.files["index.html"])


if __name__ == "__main__":
    unittest.main()
