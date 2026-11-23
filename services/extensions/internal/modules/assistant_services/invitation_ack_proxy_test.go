package assistant_services

import (
	"encoding/binary"
	"io"
	"net"
	"net/url"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// A real wire proxy drops the client's connection only after PostgreSQL has
// sent CommandComplete(COMMIT). It never fabricates a database response.
type invitationPGProxy struct {
	listener    net.Listener
	backend     string
	nth         int64
	commits     atomic.Int64
	dropped     atomic.Int64
	connections sync.Map
	wg          sync.WaitGroup
	done        chan struct{}
}

func invitationCommitProxy(t *testing.T, nth int64) (string, *invitationPGProxy) {
	t.Helper()
	path := os.Getenv("LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal("private fixture URL unavailable")
	}
	u, e := url.Parse(strings.TrimSpace(string(raw)))
	if e != nil {
		t.Fatal("fixture URL invalid")
	}
	q := u.Query()
	host, port := q.Get("host"), q.Get("port")
	if !strings.HasPrefix(host, "/home/lightjunction/.local/state/lmm-api/invitation-pg-") || port == "" || q.Get("sslmode") != "disable" {
		t.Fatal("proxy only supports this isolated Unix fixture")
	}
	listener, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	p := &invitationPGProxy{listener: listener, backend: host + "/.s.PGSQL." + port, nth: nth, done: make(chan struct{})}
	go func() {
		defer close(p.done)
		for {
			c, e := listener.Accept()
			if e != nil {
				return
			}
			p.wg.Add(1)
			p.connections.Store(c, true)
			go func() { defer p.wg.Done(); defer p.connections.Delete(c); defer c.Close(); p.serve(c) }()
		}
	}()
	t.Cleanup(func() {
		_ = listener.Close()
		<-p.done
		p.connections.Range(func(k, v any) bool { _ = k.(net.Conn).Close(); return true })
		p.wg.Wait()
	})
	u.Host = listener.Addr().String()
	q.Del("host")
	q.Del("port")
	u.RawQuery = q.Encode()
	return u.String(), p
}
func invitationFrame(r io.Reader) (byte, []byte, error) {
	head := make([]byte, 5)
	if _, e := io.ReadFull(r, head); e != nil {
		return 0, nil, e
	}
	n := int(binary.BigEndian.Uint32(head[1:]))
	if n < 4 || n > 4<<20 {
		return 0, nil, io.ErrUnexpectedEOF
	}
	rest := make([]byte, n-4)
	if _, e := io.ReadFull(r, rest); e != nil {
		return 0, nil, e
	}
	return head[0], append(head, rest...), nil
}
func (p *invitationPGProxy) serve(client net.Conn) {
	server, e := net.DialTimeout("unix", p.backend, 3*time.Second)
	if e != nil {
		return
	}
	defer server.Close()
	_ = client.SetDeadline(time.Now().Add(30 * time.Second))
	_ = server.SetDeadline(time.Now().Add(30 * time.Second))
	header := make([]byte, 4)
	if _, e = io.ReadFull(client, header); e != nil {
		return
	}
	n := int(binary.BigEndian.Uint32(header))
	if n < 8 || n > 64<<10 {
		return
	}
	startup := make([]byte, n-4)
	if _, e = io.ReadFull(client, startup); e != nil {
		return
	}
	if _, e = server.Write(append(header, startup...)); e != nil {
		return
	}
	var commit atomic.Int64
	cdone := make(chan struct{})
	go func() {
		defer close(cdone)
		defer server.Close()
		for {
			kind, frame, e := invitationFrame(client)
			if e != nil {
				return
			}
			if kind == 'Q' && strings.EqualFold(strings.TrimSpace(strings.TrimSuffix(string(frame[5:]), "\x00")), "COMMIT") {
				commit.Store(p.commits.Add(1))
			}
			if _, e = server.Write(frame); e != nil {
				return
			}
		}
	}()
	defer func() { _ = client.Close(); _ = server.Close(); <-cdone }()
	for {
		kind, frame, e := invitationFrame(server)
		if e != nil {
			return
		}
		if kind == 'C' && string(frame[5:]) == "COMMIT\x00" && commit.Load() == p.nth {
			p.dropped.Add(1)
			return
		}
		if _, e = client.Write(frame); e != nil {
			return
		}
	}
}
