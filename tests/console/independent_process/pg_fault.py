"""Local fixture transport. Never log PostgreSQL payloads or credentials.

Forward a real runtime connection to a private Unix-socket PostgreSQL. When
armed, discard COMMIT responses only for a transaction using the selected writer
(default: Console).
CommandComplete(COMMIT) and ReadyForQuery(I) must both be observed upstream.
This does not implement a database, synthesize success, or change Core.
"""

import socket
import socketserver
import struct
import threading


def exact(sock, size):
    result = bytearray()
    while len(result) < size:
        part = sock.recv(size - len(result))
        if not part:
            raise EOFError()
        result.extend(part)
    return bytes(result)


def frame(sock):
    kind = exact(sock, 1)
    length = exact(sock, 4)
    size = struct.unpack("!I", length)[0]
    if not 4 <= size <= 16 * 1024 * 1024:
        raise ValueError("protocol_frame_size")
    payload = exact(sock, size - 4)
    return kind, payload, kind + length + payload


class Proxy(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = False

    def __init__(self, unix_path, user, database, writer_sql=b"core_console.write_public_option", label="console", listen_socket=None):
        self.writer_sql, self.label = writer_sql.lower(), label
        self.unix_path, self.user, self.database = str(unix_path), user, database
        self.lock = threading.Lock()
        self.armed = False
        self.events, self.errors, self.connections = [], [], set()
        self.dropped = threading.Event()
        if listen_socket is not None:
            self.address_family = socket.AF_UNIX
        super().__init__(str(listen_socket) if listen_socket is not None else ("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    def arm(self):
        with self.lock:
            if self.armed or self.dropped.is_set():
                raise ValueError("fault_already_armed_or_used")
            self.armed = True
            self.events.append(f"armed_for_{self.label}_write_commit")

    def claim(self):
        with self.lock:
            if not self.armed:
                return False
            self.armed = False
            self.events.append(f"{self.label}_commit_forwarded")
            return True

    def handle_error(self, request, client_address):
        self.errors.append("proxy_handler_error")

    def close(self):
        self.shutdown()
        with self.lock:
            sockets = list(self.connections)
        for sock in sockets:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            sock.close()
        self.server_close()
        self.thread.join(5)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        client = self.request
        upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        upstream.connect(self.server.unix_path)
        state = {"writes": False, "drop": False, "complete": False}
        statements = {}
        with self.server.lock:
            self.server.connections.update((client, upstream))

        def frontend():
            try:
                while True:
                    kind, data, raw = frame(client)
                    sql = None
                    if kind == b"P":
                        name, sql, _ = data.split(b"\0", 2)
                        statements[name] = self.server.writer_sql in sql.lower()
                    elif kind == b"B":
                        _, name, _ = data.split(b"\0", 2)
                        state["writes"] |= statements.get(name, False)
                    elif kind == b"Q":
                        sql = data.rstrip(b"\0")
                    if sql is not None:
                        state["writes"] |= self.server.writer_sql in sql.lower()
                        if sql.strip().rstrip(b";").upper() == b"COMMIT" and state["writes"]:
                            # SQLx transaction.commit uses a simple-query message.
                            # Refuse unimplemented extended COMMIT, never claim injection.
                            if kind != b"Q":
                                self.server.errors.append("extended_commit_not_supported")
                                return
                            state["drop"] = self.server.claim()
                    upstream.sendall(raw)
            except (EOFError, OSError):
                pass
            finally:
                try:
                    upstream.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

        thread = None
        try:
            length = exact(client, 4)
            size = struct.unpack("!I", length)[0]
            if not 8 <= size <= 65536:
                raise ValueError("startup_size")
            data = exact(client, size - 4)
            if data[:4] != struct.pack("!I", 196608):
                raise ValueError("plaintext_protocol_v3_required")
            fields = data[4:].rstrip(b"\0").split(b"\0")
            params = dict(zip(fields[::2], fields[1::2]))
            if params.get(b"user") != self.server.user.encode() or params.get(b"database") != self.server.database.encode():
                raise ValueError("unexpected_runtime_identity")
            upstream.sendall(length + data)
            thread = threading.Thread(target=frontend, daemon=True)
            thread.start()
            while True:
                kind, data, raw = frame(upstream)
                if state["drop"]:
                    if kind == b"C" and data == b"COMMIT\0":
                        state["complete"] = True
                        self.server.events.append("upstream_command_complete_commit_suppressed")
                    if kind == b"Z":
                        if data == b"I" and state["complete"]:
                            self.server.events.append("upstream_ready_idle_suppressed_connection_closed")
                            self.server.dropped.set()
                        else:
                            self.server.errors.append("commit_not_confirmed_upstream")
                        break
                else:
                    if kind == b"Z" and data == b"I":
                        state["writes"] = False
                    client.sendall(raw)
        except (EOFError, OSError):
            pass
        except ValueError:
            self.server.errors.append("proxy_protocol_or_identity_error")
        finally:
            for sock in (client, upstream):
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                sock.close()
            if thread:
                thread.join(3)
            with self.server.lock:
                self.server.connections.difference_update((client, upstream))
