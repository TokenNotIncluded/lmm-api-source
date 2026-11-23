package assistant_services

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/textproto"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"
)

type invitationSMTPFixture struct {
	listener    net.Listener
	certificate tls.Certificate
	ca          []byte
	implicit    bool
	mode        string
	mu          sync.Mutex
	deliveries  int
	bodies      []string
	recipients  []string
	auths       int
	connections sync.WaitGroup
	release     <-chan struct{}
}

func newInvitationSMTPFixture(t *testing.T, implicit bool, mode string) *invitationSMTPFixture {
	t.Helper()
	key, e := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if e != nil {
		t.Fatal(e)
	}
	cert := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "Synthetic invitation SMTP"}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, IsCA: true, BasicConstraintsValid: true, IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}}
	der, e := x509.CreateCertificate(rand.Reader, cert, cert, &key.PublicKey, key)
	if e != nil {
		t.Fatal(e)
	}
	private, e := x509.MarshalECPrivateKey(key)
	if e != nil {
		t.Fatal(e)
	}
	ca := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	pair, e := tls.X509KeyPair(ca, pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: private}))
	if e != nil {
		t.Fatal(e)
	}
	listener, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	f := &invitationSMTPFixture{listener: listener, certificate: pair, ca: ca, implicit: implicit, mode: mode}
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			conn, e := listener.Accept()
			if e != nil {
				return
			}
			f.connections.Add(1)
			go func() { defer f.connections.Done(); f.serve(conn) }()
		}
	}()
	t.Cleanup(func() { _ = listener.Close(); <-done; f.connections.Wait() })
	return f
}
func (f *invitationSMTPFixture) serve(conn net.Conn) {
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(6 * time.Second))
	tlsConfig := &tls.Config{MinVersion: tls.VersionTLS12, Certificates: []tls.Certificate{f.certificate}}
	if f.implicit {
		conn = tls.Server(conn, tlsConfig)
	}
	wire := textproto.NewConn(conn)
	defer wire.Close()
	_ = wire.PrintfLine("220 synthetic SMTP")
	for {
		line, e := wire.ReadLine()
		if e != nil {
			return
		}
		switch {
		case strings.HasPrefix(line, "EHLO "):
			_ = wire.PrintfLine("250-synthetic")
			if !f.implicit {
				_ = wire.PrintfLine("250-STARTTLS")
			}
			_ = wire.PrintfLine("250 AUTH PLAIN")
		case strings.HasPrefix(line, "HELO "):
			_ = wire.PrintfLine("250 synthetic")
		case line == "STARTTLS":
			_ = wire.PrintfLine("220 begin TLS")
			conn = tls.Server(conn, tlsConfig)
			if conn.(*tls.Conn).Handshake() != nil {
				return
			}
			wire = textproto.NewConn(conn)
		case strings.HasPrefix(line, "AUTH PLAIN "):
			f.mu.Lock()
			f.auths++
			f.mu.Unlock()
			_ = wire.PrintfLine("235 authenticated")
		case strings.HasPrefix(line, "MAIL FROM:"):
			_ = wire.PrintfLine("250 sender accepted")
		case strings.HasPrefix(line, "RCPT TO:"):
			if f.mode == "reject" {
				_ = wire.PrintfLine("550 synthetic recipient rejection")
				continue
			}
			f.mu.Lock()
			f.recipients = append(f.recipients, line)
			f.mu.Unlock()
			_ = wire.PrintfLine("250 recipient accepted")
		case line == "DATA":
			_ = wire.PrintfLine("354 send body")
			body, e := io.ReadAll(wire.DotReader())
			if e != nil {
				return
			}
			f.mu.Lock()
			f.deliveries++
			f.bodies = append(f.bodies, string(body))
			release := f.release
			f.mu.Unlock()
			if release != nil {
				<-release
			}
			if f.mode == "drop_after_data" {
				return
			}
			_ = wire.PrintfLine("250 accepted")
		case line == "QUIT":
			_ = wire.PrintfLine("221 bye")
			return
		default:
			_ = wire.PrintfLine("500 unsupported command")
		}
	}
}
func (f *invitationSMTPFixture) sender(t *testing.T, auth bool) *InvitationSMTP {
	t.Helper()
	host, port, e := net.SplitHostPort(f.listener.Addr().String())
	if e != nil {
		t.Fatal(e)
	}
	p, _ := strconv.Atoi(port)
	mode := "starttls"
	if f.implicit {
		mode = "implicit"
	}
	cfg := InvitationSMTPConfig{Host: host, Port: p, TLSMode: mode, From: "sender@example.invalid", CAFile: "synthetic-ca.pem"}
	password := ""
	if auth {
		cfg.Username = "synthetic-user"
		cfg.PasswordFile = "synthetic-private.secret"
		password = "synthetic-password-never-output"
	}
	s, e := NewInvitationSMTP(cfg, password, f.ca, "https://console.example.invalid")
	if e != nil {
		t.Fatal(e)
	}
	return s
}
func TestInvitationSMTPRealTLSAcceptanceAndExactLink(t *testing.T) {
	for _, implicit := range []bool{true, false} {
		t.Run(fmt.Sprint(implicit), func(t *testing.T) {
			f := newInvitationSMTPFixture(t, implicit, "accept")
			sender := f.sender(t, true)
			got := sender.SendInvitation(context.Background(), "synthetic-operation-0001", "visitor+tag@example.invalid", "aB09")
			if got != (InvitationDelivery{"accepted_by_smtp", 250}) {
				t.Fatal("SMTP acceptance lost", got)
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			if f.deliveries != 1 || f.auths != 1 || len(f.bodies) != 1 || !strings.Contains(f.bodies[0], "https://console.example.invalid/sign-up?aff=aB09") || strings.Contains(f.bodies[0], "synthetic-password") {
				t.Fatal("wrong delivery/link or leaked secret")
			}
			if len(f.recipients) != 1 || f.recipients[0] != "RCPT TO:<visitor+tag@example.invalid>" {
				t.Fatal("recipient changed")
			}
		})
	}
}
func TestInvitationSMTPRejectionAndLostDATAAckRemainDistinct(t *testing.T) {
	for _, tc := range []struct {
		mode, state string
		code, count int
	}{{"reject", "rejected", 550, 0}, {"drop_after_data", "unknown", 0, 1}} {
		t.Run(tc.mode, func(t *testing.T) {
			f := newInvitationSMTPFixture(t, true, tc.mode)
			sender := f.sender(t, false)
			got := sender.SendInvitation(context.Background(), "synthetic-operation-0002", "visitor@example.invalid", "aB09")
			if got != (InvitationDelivery{tc.state, tc.code}) {
				t.Fatal("incorrect SMTP boundary", got)
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			if f.deliveries != tc.count {
				t.Fatal("provider retried or sent rejected recipient")
			}
		})
	}
}
func TestInvitationSMTPUnconfiguredInvalidAndSecretsStayPrivate(t *testing.T) {
	if (InvitationSMTPConfig{}).Enabled() {
		t.Fatal("empty SMTP enabled")
	}
	f := newInvitationSMTPFixture(t, true, "accept")
	s := f.sender(t, false)
	for _, recipient := range []string{"Display <v@example.invalid>", "v@example.invalid\r\nBcc: x@example.invalid", "UPPER@example.invalid"} {
		if got := s.SendInvitation(context.Background(), "synthetic-operation-0003", recipient, "aB09"); got.State != "not_dispatched" {
			t.Fatal("unsafe recipient reached SMTP", got)
		}
	}
	if _, e := NewInvitationSMTP(s.config, "not-configured-secret", f.ca, "https://console.example.invalid"); e == nil {
		t.Fatal("orphan password accepted")
	}
	if _, e := NewInvitationSMTP(s.config, "", f.ca, "http://console.example.invalid"); e == nil {
		t.Fatal("insecure signup origin accepted")
	}
	if _, e := NewInvitationSMTP(s.config, "", f.ca, "https://evil.example.invalid/path"); e == nil {
		t.Fatal("arbitrary path origin accepted")
	}
	bad := s.config
	bad.TLSMode = "plain"
	if bad.Validate() == nil {
		t.Fatal("plaintext SMTP accepted")
	}
	bad = s.config
	bad.Host = "127.0.0.1:25"
	if bad.Validate() == nil {
		t.Fatal("host/port authority ambiguous")
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.deliveries != 0 {
		t.Fatal("invalid input sent mail")
	}
}
