"""Owned local bridge around the repository's existing real COMMIT-ACK proxy.
No credentials/payload logging; only numeric loopback database transport.
"""
import importlib.util
import json
import socket
import socketserver
import sys
import threading
import time
from pathlib import Path

config = json.loads(Path(sys.argv[1]).read_text())
module_path = Path(config['proxy_module'])
spec = importlib.util.spec_from_file_location('reviewed_pg_fault', module_path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class Bridge(socketserver.ThreadingUnixStreamServer):
    daemon_threads = True
class Forward(socketserver.BaseRequestHandler):
    def handle(self):
        upstream = socket.create_connection(('127.0.0.1', config['upstream_port']), timeout=5)
        upstream.settimeout(None)
        upstream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        def copy(source, target):
            try:
                while True:
                    data = source.recv(65536)
                    if not data:
                        break
                    target.sendall(data)
            except OSError:
                pass
            finally:
                try:
                    target.shutdown(socket.SHUT_WR)
                except OSError:
                    pass
        worker = threading.Thread(target=copy, args=(self.request, upstream), daemon=True)
        worker.start()
        copy(upstream, self.request)
        upstream.close()
        worker.join(3)

socket_path = Path(config['socket_dir'])/'upstream.sock'
bridge = Bridge(str(socket_path), Forward)
bridge_thread = threading.Thread(target=bridge.serve_forever, daemon=True)
bridge_thread.start()
class NoDelay(module.Handler):
    def setup(self):
        self.request.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        super().setup()
module.Handler = NoDelay

proxy = module.Proxy(socket_path, config['user'], config['database'],
                     writer_sql=config['writer'].encode(), label='continuity')
proxy.arm()
def record():
    path = Path(config['events'])
    pending = path.with_suffix('.new')
    pending.write_text(json.dumps({'events':proxy.events, 'errors':proxy.errors,
                                  'dropped':proxy.dropped.is_set()}))
    pending.replace(path)
Path(config['ready']).write_text(str(proxy.server_address[1]))
try:
    while not Path(config['stop']).exists():
        if proxy.dropped.is_set():
            record()
        time.sleep(.02)
finally:
    proxy.close()
    record()
    bridge.shutdown()
    bridge.server_close()
    bridge_thread.join(3)
    socket_path.unlink(missing_ok=True)
