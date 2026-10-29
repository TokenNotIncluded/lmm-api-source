#!/usr/bin/env python3
"""Injector self-check only. Synthetic frames are NOT Core/PG acceptance."""

from pathlib import Path
import socket
import struct
import tempfile
import threading
import unittest

from pg_fault import Proxy, exact, frame


def packet(kind, payload):
    return kind + struct.pack("!I", len(payload) + 4) + payload


class InjectorTest(unittest.TestCase):
    def test_passthrough_and_targeted_commit_response_loss(self):
        with tempfile.TemporaryDirectory(prefix="lmm-proxy-self-") as directory:
            unix_path = Path(directory) / "pg"
            backend = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            backend.bind(str(unix_path))
            backend.listen(1)
            failures, received = [], []

            def fixture():
                try:
                    conn, _ = backend.accept()
                    with conn:
                        conn.settimeout(5)
                        size = struct.unpack("!I", exact(conn, 4))[0]
                        exact(conn, size - 4)
                        conn.sendall(packet(b"R", struct.pack("!I", 0)) + packet(b"Z", b"I"))
                        while True:
                            kind, data, _ = frame(conn)
                            if kind == b"Q":
                                received.append(data)
                                if data == b"COMMIT\0":
                                    conn.sendall(packet(b"C", b"COMMIT\0") + packet(b"Z", b"I"))
                                elif data == b"BEGIN\0":
                                    conn.sendall(packet(b"C", b"BEGIN\0") + packet(b"Z", b"T"))
                            elif kind == b"P":
                                conn.sendall(packet(b"1", b""))
                            elif kind == b"B":
                                conn.sendall(packet(b"2", b""))
                            elif kind == b"S":
                                conn.sendall(packet(b"Z", b"T"))
                except EOFError:
                    pass
                except Exception as error:
                    failures.append(type(error).__name__)

            worker = threading.Thread(target=fixture, daemon=True)
            worker.start()
            proxy = Proxy(unix_path, "runtime", "testdb")
            client = socket.create_connection(proxy.server_address, timeout=5)
            try:
                startup = struct.pack("!I", 196608) + b"user\0runtime\0database\0testdb\0\0"
                client.sendall(struct.pack("!I", len(startup) + 4) + startup)
                self.assertEqual(frame(client)[:2], (b"R", struct.pack("!I", 0)))
                self.assertEqual(frame(client)[:2], (b"Z", b"I"))

                def query(sql, tag, ready):
                    client.sendall(packet(b"Q", sql + b"\0"))
                    self.assertEqual(frame(client)[:2], (b"C", tag + b"\0"))
                    self.assertEqual(frame(client)[:2], (b"Z", ready))

                # Teach the proxy a prepared writer, then complete normally.
                query(b"BEGIN", b"BEGIN", b"T")
                client.sendall(packet(b"P", b"writer\0SELECT core_console.write_public_option($1,$2)\0\0\0"))
                self.assertEqual(frame(client)[:2], (b"1", b""))
                query(b"COMMIT", b"COMMIT", b"I")
                proxy.arm()
                # An unrelated transaction must NOT consume the armed fault.
                query(b"BEGIN", b"BEGIN", b"T")
                query(b"COMMIT", b"COMMIT", b"I")
                self.assertFalse(proxy.dropped.is_set())
                self.assertTrue(proxy.armed)
                # Reuse the prepared writer without another Parse frame.
                query(b"BEGIN", b"BEGIN", b"T")
                client.sendall(packet(b"B", b"\0writer\0\0\0\0\0\0\0"))
                self.assertEqual(frame(client)[:2], (b"2", b""))
                client.sendall(packet(b"Q", b"COMMIT\0"))
                self.assertEqual(client.recv(1), b"")
                self.assertTrue(proxy.dropped.wait(2))
                self.assertEqual(proxy.events, ["armed_for_console_write_commit", "console_commit_forwarded",
                    "upstream_command_complete_commit_suppressed", "upstream_ready_idle_suppressed_connection_closed"])
                self.assertEqual(proxy.errors, [])
                self.assertEqual(received.count(b"COMMIT\0"), 3)
            finally:
                client.close()
                proxy.close()
                backend.close()
                worker.join(5)
            self.assertFalse(worker.is_alive())
            self.assertEqual(failures, [])


if __name__ == "__main__":
    unittest.main()
