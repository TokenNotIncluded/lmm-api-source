"""Pause one actual management ALTER DATABASE acknowledgement on a Unix socket.

It is a forwarding observer, not a PostgreSQL implementation. It signals only
when the upstream has sent CommandComplete and ReadyForQuery idle. No protocol
payloads or passwords are logged.
"""
import socket
import socketserver
import struct
import threading


def exact(sock, length):
    out=bytearray()
    while len(out)<length:
        part=sock.recv(length-len(out))
        if not part:raise EOFError()
        out.extend(part)
    return bytes(out)


def frame(sock):
    kind=exact(sock,1); length=exact(sock,4); n=struct.unpack('!I',length)[0]
    if not 4<=n<=16*1024*1024:raise ValueError('invalid_frame_length')
    data=exact(sock,n-4)
    return kind,data,kind+length+data


class ThreadedUnix(socketserver.ThreadingMixIn,socketserver.UnixStreamServer):
    daemon_threads=True


class ManagementPause(ThreadedUnix):
    def __init__(self,listen_socket,upstream):
        self.upstream=str(upstream);self.hit=threading.Event();self.release=threading.Event()
        self.events=[];self.errors=[];self.lock=threading.Lock();self.sockets=set()
        super().__init__(str(listen_socket),Handler)
        self.thread=threading.Thread(target=self.serve_forever,daemon=True);self.thread.start()
    def handle_error(self,request,address):self.errors.append('handler_error')
    def close(self):
        self.release.set();self.shutdown()
        with self.lock:sockets=list(self.sockets)
        for s in sockets:
            try:s.shutdown(socket.SHUT_RDWR)
            except OSError:pass
            s.close()
        self.server_close();self.thread.join(5)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        client=self.request;upstream=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
        upstream.connect(self.server.upstream);state={'target':False,'withhold':False};statements={}
        with self.server.lock:self.server.sockets.update((client,upstream))
        def target(sql):
            normal=b' '.join(sql.lower().split())
            return b'alter database' in normal and b'allow_connections true' in normal
        def frontend():
            try:
                while True:
                    kind,data,raw=frame(client)
                    if kind==b'P':
                        name,sql,_=data.split(b'\0',2);statements[name]=target(sql)
                    elif kind==b'B':
                        _,name,_=data.split(b'\0',2);state['target'] |= statements.get(name,False)
                    elif kind==b'Q':state['target'] |= target(data.rstrip(b'\0'))
                    upstream.sendall(raw)
            except (EOFError,OSError):pass
            except ValueError:self.server.errors.append('frontend_invalid_frame')
        thread=None
        try:
            length=exact(client,4);size=struct.unpack('!I',length)[0]
            if not 8<=size<=65536:raise ValueError('startup_size')
            data=exact(client,size-4)
            if data[:4]!=struct.pack('!I',196608):raise ValueError('protocol_not_plain_v3')
            fields=data[4:].rstrip(b'\0').split(b'\0');params=dict(zip(fields[::2],fields[1::2]))
            if params.get(b'user')!=b'postgres' or params.get(b'database')!=b'postgres':raise ValueError('wrong_management_identity')
            upstream.sendall(length+data);thread=threading.Thread(target=frontend,daemon=True);thread.start()
            while True:
                kind,data,raw=frame(upstream)
                if state['target'] and kind==b'C' and data==b'ALTER DATABASE\0':
                    state['withhold']=True;self.server.events.append('alter_database_command_complete_observed')
                if state['withhold']:
                    if kind==b'Z':
                        if data==b'I':
                            self.server.events.append('alter_database_ready_idle_observed_ack_held');self.server.hit.set()
                            self.server.release.wait(120)
                        else:self.server.errors.append('unexpected_transaction_state')
                        break
                else:
                    client.sendall(raw)
                    if kind==b'Z':state['target']=False
        except (EOFError,OSError):pass
        except ValueError:self.server.errors.append('protocol_or_identity_error')
        finally:
            for s in (client,upstream):
                try:s.shutdown(socket.SHUT_RDWR)
                except OSError:pass
                s.close()
            if thread:thread.join(3)
            with self.server.lock:self.server.sockets.difference_update((client,upstream))
