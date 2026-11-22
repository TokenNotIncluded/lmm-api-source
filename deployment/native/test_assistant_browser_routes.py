#!/usr/bin/env python3
"""Validate exact native browser routes with isolated Nginx and synthetic HTTP.

No Core, PostgreSQL, browser cookie jar, SMTP, coupon issuance or production
configuration is involved. Header forwarding is not an authorization test.
"""
import http.client
import http.server
import json
import pathlib
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
TEMPLATE = ROOT / "deployment/native/templates/native-assistant-browser-locations.conf"
MANAGEMENT = ROOT / "deployment/native/templates/native-management-locations.conf"
CURRENT = "/api/user/auth/assistant/confirmations"
LEGACY = "/api/user/assistant/confirmations"
WEEKLY = "/api/user/auth/assistant/weekly-discounts"
IDENTIFIER = "a" * 64


def backend():
    class Handler(http.server.BaseHTTPRequestHandler):
        def handle_call(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            value = {
                "method": self.command, "path": self.path, "body": body.decode(),
                "cookie": self.headers.get("Cookie"),
                "origin": self.headers.get("Origin"),
                "csrf": self.headers.get("X-LMM-Assistant-CSRF"),
                "host": self.headers.get("Host"),
                "forwarded_proto": self.headers.get("X-Forwarded-Proto"),
                "canary": self.headers.get("X-LMM-Canary"),
            }
            self.server.calls.append(value)
            raw = json.dumps(value).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(raw)

        do_GET = do_HEAD = do_POST = do_PATCH = handle_call

        def log_message(self, *_args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.calls = []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


@unittest.skipUnless(shutil.which("nginx"), "local nginx is required")
class NativeAssistantBrowserRoutes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="lmm-browser-routes-")
        cls.addClassCleanup(cls.directory.cleanup)
        cls.path = pathlib.Path(cls.directory.name)
        cls.core = backend()
        cls.addClassCleanup(cls.core.server_close)
        cls.addClassCleanup(cls.core.shutdown)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            cls.port = listener.getsockname()[1]
        for name, source in [("browser.conf", TEMPLATE), ("management.conf", MANAGEMENT)]:
            (cls.path / name).write_text(
                source.read_text().replace("127.0.0.1:3101", f"127.0.0.1:{cls.core.server_port}")
            )
        config = (
            f"daemon off; master_process off; error_log {cls.path}/error.log notice; "
            f"pid {cls.path}/nginx.pid; events {{worker_connections 64;}} "
            f"http {{access_log off; client_body_temp_path {cls.path}/body; "
            f"proxy_temp_path {cls.path}/proxy; fastcgi_temp_path {cls.path}/fastcgi; "
            f"uwsgi_temp_path {cls.path}/uwsgi; scgi_temp_path {cls.path}/scgi; "
            f"server {{listen 127.0.0.1:{cls.port}; server_name native.example.invalid; root {cls.path}; "
            f"include {cls.path}/management.conf; include {cls.path}/browser.conf; "
            "location @lmm_extensions_backend {return 418;} "
            "location ^~ /api/user/auth/ {return 418;} "
            "location ^~ /api/user/ {return 418;} "
            "location ^~ /core/ {return 418;} location / {return 418;} } }"
        )
        (cls.path / "nginx.conf").write_text(config)
        args = [shutil.which("nginx"), "-p", str(cls.path), "-c", str(cls.path / "nginx.conf")]
        checked = subprocess.run(args + ["-t"], capture_output=True)
        if checked.returncode:
            raise RuntimeError(checked.stderr.decode())
        cls.process = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        cls.addClassCleanup(cls.stop_nginx)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                if cls.request("GET", "/")[0] == 418:
                    return
            except OSError:
                pass
            if cls.process.poll() is not None:
                raise RuntimeError("isolated nginx exited")
            time.sleep(0.03)
        raise RuntimeError("isolated nginx did not start")

    @classmethod
    def stop_nginx(cls):
        if cls.process.poll() is None:
            cls.process.send_signal(signal.SIGQUIT)
        cls.process.communicate(timeout=5)

    @classmethod
    def request(cls, method, path, body=None):
        connection = http.client.HTTPConnection("127.0.0.1", cls.port, timeout=3)
        headers = {
            "Host": "native.example.invalid",
            "Cookie": "new_api_refresh=synthetic-session; __Host-lmm-weekly-fixture=synthetic-nonce",
            "Origin": "https://native.example.invalid",
            "X-LMM-Assistant-CSRF": "synthetic-nonce",
            "Content-Type": "application/json",
            "X-LMM-Canary": "must-not-forward",
        }
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        status, raw = response.status, response.read()
        connection.close()
        return status, raw

    def test_exact_paths_preserve_browser_headers_method_uri_and_body(self):
        for method, path in [
            ("GET", WEEKLY),
            ("GET", WEEKLY + "?before_week=345600"),
            ("GET", WEEKLY + "/" + IDENTIFIER),
            ("POST", WEEKLY + "/" + IDENTIFIER),
            ("GET", CURRENT + "/ui.js"),
            ("GET", CURRENT + "/" + IDENTIFIER),
            ("POST", CURRENT + "/" + IDENTIFIER),
            ("GET", CURRENT + "/" + IDENTIFIER + "/key"),
            ("POST", CURRENT + "/" + IDENTIFIER + "/key"),
            ("GET", LEGACY + "/ui.js"),
            ("GET", LEGACY + "/" + IDENTIFIER),
            ("GET", LEGACY + "/" + IDENTIFIER + "/key"),
        ]:
            with self.subTest(method=method, path=path):
                body = '{"claim":true}' if method == "POST" else None
                status, raw = self.request(method, path, body)
                self.assertEqual(status, 200)
                actual = json.loads(raw)
                self.assertEqual(actual["method"], method)
                self.assertEqual(actual["path"], path)
                self.assertEqual(actual["body"], body or "")
                self.assertEqual(actual["origin"], "https://native.example.invalid")
                self.assertEqual(actual["csrf"], "synthetic-nonce")
                self.assertEqual(actual["cookie"], "new_api_refresh=synthetic-session; __Host-lmm-weekly-fixture=synthetic-nonce")
                self.assertEqual(actual["host"], "native.example.invalid")
                self.assertEqual(actual["forwarded_proto"], "https")
                self.assertIsNone(actual["canary"])

    def test_old_posts_and_other_methods_never_reach_core(self):
        before = len(self.core.calls)
        for method, path in [
            ("POST", WEEKLY),
            ("PATCH", WEEKLY + "/" + IDENTIFIER),
            ("POST", CURRENT + "/ui.js"),
            ("PATCH", CURRENT + "/" + IDENTIFIER),
            ("POST", LEGACY + "/ui.js"),
            ("POST", LEGACY + "/" + IDENTIFIER),
            ("POST", LEGACY + "/" + IDENTIFIER + "/key"),
        ]:
            with self.subTest(method=method, path=path):
                self.assertEqual(self.request(method, path, '{"claim":true}')[0], 405)
        self.assertEqual(len(self.core.calls), before)

    def test_raw_uri_guard_rejects_normalization_and_neighbor_paths(self):
        before = len(self.core.calls)
        for prefix in [CURRENT, LEGACY, WEEKLY]:
            for suffix in [
                "/" + "A" * 64, "/" + "a" * 63, "/" + "a" * 65,
                "/%61" + "a" * 63, "//" + IDENTIFIER,
                "/other/../" + IDENTIFIER, "/%2e/" + IDENTIFIER,
                "/" + IDENTIFIER + "/extra", "/" + IDENTIFIER + "?claim=true",
                "/%2f" + IDENTIFIER, "/%5c" + IDENTIFIER,
            ]:
                with self.subTest(path=prefix + suffix):
                    self.assertEqual(self.request("GET", prefix + suffix)[0], 404)
        for path in [
            CURRENT, LEGACY, CURRENT + "/ui.js?extra=1",
            LEGACY + "/ui.js?extra=1", WEEKLY + "/" + IDENTIFIER + "/key",
        ]:
            with self.subTest(path=path):
                self.assertEqual(self.request("GET", path)[0], 404)
        self.assertEqual(len(self.core.calls), before)

    def test_unrelated_auth_and_core_routes_keep_existing_policy(self):
        before = len(self.core.calls)
        for path in ["/api/user/auth/refresh", "/api/user/auth/assistant/other",
                     "/api/user/auth/assistant/weekly-discounts-extra", "/core/other"]:
            with self.subTest(path=path):
                self.assertEqual(self.request("GET", path)[0], 418)
        self.assertEqual(len(self.core.calls), before)

    def test_head_keeps_get_route_semantics(self):
        for path in [WEEKLY, WEEKLY + "/" + IDENTIFIER, CURRENT + "/ui.js",
                     CURRENT + "/" + IDENTIFIER + "/key", LEGACY + "/" + IDENTIFIER]:
            with self.subTest(path=path):
                status, raw = self.request("HEAD", path)
                self.assertEqual(status, 200)
                self.assertEqual(raw, b"")
                self.assertEqual(self.core.calls[-1]["method"], "HEAD")


if __name__ == "__main__":
    unittest.main()
