#!/usr/bin/env python3
"""Exercise the native management route snippet against local HTTP stubs."""
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
TEMPLATE = ROOT / 'deployment/native/templates/native-management-locations.conf'
RANKINGS = ROOT / 'deployment/native/templates/native-rankings-locations.conf'


def backend(name):
    class Handler(http.server.BaseHTTPRequestHandler):
        def handle_call(self):
            body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
            value = dict(backend=name, method=self.command, path=self.path,
                         authorization=self.headers.get('Authorization'), body=body.decode(),
                         policy_path=self.headers.get('X-Test-Policy-Path'))
            self.server.calls.append(value)
            raw = json.dumps(value).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(raw)))
            self.end_headers()
            if self.command != 'HEAD':
                self.wfile.write(raw)

        do_GET = do_HEAD = do_PATCH = do_PUT = do_POST = handle_call

        def log_message(self, *_args):
            pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.calls = []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


@unittest.skipUnless(shutil.which('nginx'), 'local nginx is required')
class NativeManagementRoutes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix='lmm-management-routes-')
        cls.path = pathlib.Path(cls.directory.name)
        cls.core, cls.go = backend('core'), backend('go')
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            cls.port = listener.getsockname()[1]
        snippet = TEMPLATE.read_text().replace('127.0.0.1:3101', f'127.0.0.1:{cls.core.server_port}')
        (cls.path / 'routes.conf').write_text(snippet + '\n' + RANKINGS.read_text())
        config = (
            f'daemon off; master_process off; error_log {cls.path}/error.log notice; '
            f'pid {cls.path}/nginx.pid; events {{worker_connections 64;}} '
            f'http {{access_log off; client_body_temp_path {cls.path}/client-body; '
            f'proxy_temp_path {cls.path}/proxy; fastcgi_temp_path {cls.path}/fastcgi; '
            f'uwsgi_temp_path {cls.path}/uwsgi; scgi_temp_path {cls.path}/scgi; '
            f'server {{listen 127.0.0.1:{cls.port}; '
            f'root {cls.path}; include {cls.path}/routes.conf; '
            'set $lmm_access_policy_original_uri 0; '
            'location @lmm_extensions_backend {proxy_set_header X-Test-Policy-Path $lmm_access_policy_original_uri; '
            f'proxy_pass http://127.0.0.1:{cls.go.server_port};}} '
            'location ^~ /core/ {return 503;} location / {return 418;} } }')
        (cls.path / 'nginx.conf').write_text(config)
        args = [shutil.which('nginx'), '-p', str(cls.path), '-c', str(cls.path / 'nginx.conf')]
        checked = subprocess.run(args + ['-t'], capture_output=True)
        if checked.returncode:
            raise RuntimeError(checked.stderr.decode())
        cls.process = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                if cls.request('GET', '/')[0] == 418:
                    return
            except OSError:
                pass
            if cls.process.poll() is not None:
                raise RuntimeError('fixture nginx exited')
            time.sleep(0.03)
        raise RuntimeError('fixture nginx did not start')

    @classmethod
    def tearDownClass(cls):
        cls.process.send_signal(signal.SIGQUIT)
        cls.process.communicate(timeout=5)
        for server in (cls.core, cls.go):
            server.shutdown()
            server.server_close()
        cls.directory.cleanup()

    @classmethod
    def request(cls, method, path, body=None, *, headers=None):
        connection = http.client.HTTPConnection('127.0.0.1', cls.port, timeout=3)
        if headers is None:
            headers = {'Authorization':'Bearer synthetic-native-route-session',
                       'Content-Type':'application/json'}
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        status, raw = response.status, response.read()
        connection.close()
        return status, raw

    def test_exact_routes_preserve_method_uri_session_and_body(self):
        for method, path, target in (
            ('GET', '/api/option/epay/config', 'go'),
            ('GET', '/api/admin/acquisition/report?from=1&to=2', 'go'),
            ('GET', '/api/admin/acquisition/funnel', 'go'),
            ('GET', '/api/admin/acquisition/visitors', 'go'),
            ('GET', '/api/admin/acquisition/links', 'go'),
            ('GET', '/api/option/payment-gateways/config', 'go'),
            ('POST', '/api/option/waffo-pancake/catalog', 'go'),
            ('POST', '/api/option/waffo-pancake/pair', 'go'),
            ('POST', '/api/option/waffo-pancake/save', 'go'),
            ('GET', '/api/option/waffo-pancake/config', 'go'),
            ('GET', '/api/rankings?period=week', 'go'),
            ('GET', '/api/rankings/users?period=week', 'go'),
            ('GET', '/core/v1/admin/user-access/17?view=current', 'core'),
            ('PATCH', '/core/v1/admin/user-access/17', 'core'),
            ('GET', '/core/v1/payment-gateways/epay', 'core'),
            ('PUT', '/core/v1/payment-gateways/epay/5004', 'core'),
        ):
            with self.subTest(method=method, path=path):
                body = '{"expected_revision":"synthetic"}' if method in ('PATCH','PUT','POST') else None
                status, raw = self.request(method, path, body)
                self.assertEqual(status, 200)
                actual = json.loads(raw)
                self.assertEqual(actual['backend'], target)
                self.assertEqual(actual['method'], method)
                self.assertEqual(actual['path'], path)
                self.assertEqual(actual['body'], body or '')
                self.assertEqual(actual['authorization'], 'Bearer synthetic-native-route-session')

    def test_public_legal_documents_preserve_method_policy_path_and_language(self):
        for path in ('/api/user-agreement', '/api/privacy-policy'):
            for method in ('GET', 'HEAD'):
                for query in ('', '?lang=zh'):
                    with self.subTest(path=path, method=method, query=query):
                        before = len(self.go.calls)
                        status, raw = self.request(method, path + query, headers={})
                        self.assertEqual(status, 200)
                        self.assertEqual(len(self.go.calls), before + 1)
                        call = self.go.calls[-1]
                        self.assertEqual((call['method'], call['path'], call['policy_path']),
                                         (method, path + query, path))
                        self.assertIsNone(call['authorization'])
                        self.assertEqual(raw, b'' if method == 'HEAD' else json.dumps(call).encode())

            before = len(self.go.calls)
            for method in ('POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'):
                with self.subTest(path=path, method=method):
                    self.assertEqual(self.request(method, path, headers={})[0], 405)
            for other in (path + '/', path + '/preview', path + '-archive', path[4:]):
                with self.subTest(path=other):
                    self.assertEqual(self.request('GET', other, headers={})[0], 418)
            self.assertEqual(len(self.go.calls), before)

    def test_unrelated_methods_ids_and_paths_are_not_forwarded(self):
        for method, path, expected in (
            ('PATCH', '/api/option/epay/config', 405),
            ('PUT', '/api/option/payment-gateways/config', 405),
            ('POST', '/api/rankings', 405),
            ('POST', '/api/rankings/users', 405),
            ('GET', '/api/rankings/other', 418),
            ('GET', '/rankings', 418),
            ('GET', '/api/option/waffo-pancake/catalog', 405),
            ('GET', '/api/option/waffo-pancake/pair', 405),
            ('GET', '/api/option/waffo-pancake/save', 405),
            ('POST', '/api/option/waffo-pancake/config', 405),
            ('GET', '/api/option/waffo-pancake/other', 418),
            ('GET', '/core/v1/admin/user-access', 404),
            ('POST', '/core/v1/admin/user-access/17', 405),
            ('GET', '/core/v1/admin/user-access/0', 404),
            ('GET', '/core/v1/admin/user-access/017', 404),
            ('GET', '/core/v1/admin/user-access/%31', 404),
            ('PATCH', '/core/v1/admin/user-access/17/roles', 404),
            ('PUT', '/core/v1/payment-gateways/epay', 405),
            ('GET', '/core/v1/payment-gateways/epay/5004', 405),
            ('PUT', '/core/v1/payment-gateways/epay/5004/enable', 404),
            ('PUT', '/core/v1/payment-gateways/epay/0', 404),
            ('GET', '/core/v1/admin/users', 503),
            ('GET', '/core/v1/payment-gateways/stripe', 503),
            ('GET', '/api/option/epay/other', 418),
        ):
            with self.subTest(method=method, path=path):
                self.assertEqual(self.request(method, path)[0], expected)


if __name__ == '__main__':
    unittest.main()
