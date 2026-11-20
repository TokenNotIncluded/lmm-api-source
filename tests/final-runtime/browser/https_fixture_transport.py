"""Closed local CONNECT + TLS gateway for real Extore/LMM acceptance.
Only the two supplied canonical hostnames can reach literal loopback targets.
This layer forwards bytes and HTTP responses; it supplies no business responses.
"""
from __future__ import annotations
import base64
import hashlib
import http.client
import http.server
import json
from pathlib import Path
import select
import socket
import ssl
import subprocess
import threading
import time

HOSTS = ("extore.lmm.best", "native.api.lmm.best")

def certificates(root: Path) -> dict:
    root.mkdir(mode=0o700)
    ca_key, ca, key, csr, cert = [root / n for n in ("ca.key", "ca.pem", "server.key", "server.csr", "server.pem")]
    commands = [
        ["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(ca_key)],
        ["openssl", "req", "-x509", "-new", "-key", str(ca_key), "-sha256", "-days", "2",
         "-subj", "/CN=LMM isolated browser acceptance CA", "-addext", "basicConstraints=critical,CA:TRUE",
         "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-addext", "subjectKeyIdentifier=hash", "-addext", "authorityKeyIdentifier=keyid:always", "-out", str(ca)],
        ["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(key)],
        ["openssl", "req", "-new", "-key", str(key), "-subj", "/CN=native.api.lmm.best", "-out", str(csr)],
    ]
    ext = root / "leaf.cnf"
    ext.write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\nsubjectAltName=" + ",".join("DNS:" + h for h in HOSTS) + "\n")
    commands.append(["openssl", "x509", "-req", "-in", str(csr), "-CA", str(ca), "-CAkey", str(ca_key),
                     "-CAcreateserial", "-out", str(cert), "-days", "2", "-sha256", "-extfile", str(ext)])
    for command in commands:
        subprocess.run(command, check=True, capture_output=True, timeout=20)
    for hostname in HOSTS:
        subprocess.run(["openssl", "verify", "-x509_strict", "-verify_hostname", hostname, "-CAfile", str(ca), str(cert)], check=True, capture_output=True, timeout=10)
    for path in root.iterdir():
        path.chmod(0o600)
    pub = subprocess.run(["openssl", "x509", "-in", str(cert), "-pubkey", "-noout"], check=True, capture_output=True).stdout
    der = subprocess.run(["openssl", "pkey", "-pubin", "-outform", "DER"], input=pub, check=True, capture_output=True).stdout
    return {"ca": str(ca), "key": str(key), "cert": str(cert),
            "ca_sha256": hashlib.sha256(ca.read_bytes()).hexdigest(),
            "certificate_sha256": hashlib.sha256(cert.read_bytes()).hexdigest(),
            "spki": base64.b64encode(hashlib.sha256(der).digest()).decode()}

class LocalHTTPS:
    def __init__(self, targets: dict[str, tuple[str, int]], cert: dict, log_path: Path):
        if set(targets) != set(HOSTS) or any(host != "127.0.0.1" or not 0 < port < 65536 for host, port in targets.values()):
            raise ValueError("both canonical hosts must have literal loopback targets")
        self.targets = dict(targets)
        self.events = []
        self.lock = threading.Lock()
        self.sockets = set()
        self.log_path = log_path
        self.log = log_path.open("x")
        self.closed = False
        owner = self

        class Server(http.server.ThreadingHTTPServer):
            daemon_threads = True
            allow_reuse_address = False

        class Gateway(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def log_message(self, *_): pass
            def do_GET(self): self.forward()
            def do_POST(self): self.forward()
            def do_PUT(self): self.forward()
            def do_PATCH(self): self.forward()
            def do_DELETE(self): self.forward()
            def do_OPTIONS(self): self.forward()
            def do_HEAD(self): self.forward()
            def forward(self):
                host = self.headers.get("Host", "").lower()
                if host.endswith(":443"): host = host[:-4]
                if host not in owner.targets:
                    self.send_error(421); self.close_connection = True
                    owner.record({"layer": "gateway", "host": host[:100], "status": 421})
                    return
                if self.headers.get("Transfer-Encoding"):
                    self.send_error(501); self.close_connection = True
                    owner.record({"layer": "gateway", "host": host, "status": 501, "reason": "chunked_request_unsupported"})
                    return
                length = int(self.headers.get("Content-Length", "0"))
                if length < 0 or length > 32 * 1024 * 1024:
                    self.send_error(413); self.close_connection = True
                    return
                body = self.rfile.read(length)
                headers = {k: v for k, v in self.headers.items()
                           if k.lower() not in ("connection", "proxy-connection", "proxy-authorization", "transfer-encoding", "accept-encoding")}
                headers["Host"] = host
                connection = http.client.HTTPConnection(*owner.targets[host], timeout=25)
                try:
                    connection.request(self.command, self.path, body=body, headers=headers)
                    response = connection.getresponse()
                    raw = response.read(32 * 1024 * 1024 + 1)
                    if len(raw) > 32 * 1024 * 1024:
                        raise ValueError("fixture forwarding response exceeded limit")
                    status, reply_headers = response.status, response.getheaders()
                except (OSError, http.client.HTTPException, ValueError) as issue:
                    status, raw, reply_headers = 502, b"Local test transport unavailable", [("Content-Type", "text/plain")]
                    owner.record({"layer": "gateway_error", "host": host, "type": type(issue).__name__})
                finally:
                    connection.close()
                owner.record({"layer": "gateway", "host": host, "method": self.command,
                              "path": self.path.split("?", 1)[0], "status": status,
                              "target": list(owner.targets[host]), "response_bytes": len(raw)})
                self.send_response(status)
                for key, value in reply_headers:
                    if key.lower() not in ("connection", "transfer-encoding", "content-length"):
                        self.send_header(key, value)
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                if self.command != "HEAD":
                    try: self.wfile.write(raw)
                    except (BrokenPipeError, ConnectionResetError): pass

        self.gateway = Server(("127.0.0.1", 0), Gateway)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.minimum_version = ssl.TLSVersion.TLSv1_2
        tls.load_cert_chain(cert["cert"], cert["key"])
        def server_name(_socket, name, _context):
            if name not in owner.targets:
                raise ssl.SSLError("unconfigured fixture server name")
        tls.set_servername_callback(server_name)
        self.gateway.socket = tls.wrap_socket(self.gateway.socket, server_side=True)
        gateway_port = self.gateway.server_address[1]

        class Proxy(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def log_message(self, *_): pass
            def do_CONNECT(self):
                authority = self.path.lower()
                if authority not in {host + ":443" for host in owner.targets}:
                    self.send_error(403)
                    self.close_connection = True
                    owner.record({"layer": "proxy", "authority": authority[:150], "status": 403, "public_fallback": False})
                    return
                upstream = socket.create_connection(("127.0.0.1", gateway_port), timeout=10)
                with owner.lock:
                    owner.sockets.update((self.connection, upstream))
                self.send_response(200, "Connection Established")
                self.end_headers()
                self.close_connection = True
                owner.record({"layer": "proxy", "authority": authority, "status": 200,
                              "target": ["127.0.0.1", gateway_port], "public_fallback": False})
                try:
                    while not owner.closed:
                        readable, _, _ = select.select([self.connection, upstream], [], [], 30)
                        if not readable: break
                        for source in readable:
                            data = source.recv(65536)
                            if not data: return
                            target = upstream if source is self.connection else self.connection
                            target.sendall(data)
                except (OSError, ValueError):
                    pass
                finally:
                    with owner.lock:
                        owner.sockets.discard(self.connection)
                        owner.sockets.discard(upstream)
                    upstream.close()
            def do_GET(self):
                self.send_error(405)
                self.close_connection = True
                owner.record({"layer": "proxy", "method": "GET", "status": 405, "public_fallback": False})

        self.proxy = Server(("127.0.0.1", 0), Proxy)
        self.proxy_url = "http://127.0.0.1:" + str(self.proxy.server_address[1])
        self.threads = []
        for server in (self.gateway, self.proxy):
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.threads.append(thread)

    def record(self, event):
        event = {"time": time.time(), **event}
        with self.lock:
            self.events.append(event)
            self.log.write(json.dumps(event) + "\n")
            self.log.flush()

    def close(self):
        self.closed = True
        with self.lock:
            active = list(self.sockets)
        for connection in active:
            try: connection.shutdown(socket.SHUT_RDWR)
            except OSError: pass
        for server in (self.proxy, self.gateway):
            server.shutdown()
            server.server_close()
        for thread in self.threads:
            thread.join(timeout=3)
        self.log.close()
