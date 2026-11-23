package assistant_services

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"net"
	"net/smtp"
	"net/textproto"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/invitationfacts"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type InvitationSMTPConfig struct {
	Host         string `toml:"smtp_host"`
	Port         int    `toml:"smtp_port"`
	TLSMode      string `toml:"tls_mode"`
	From         string `toml:"from"`
	Username     string `toml:"username"`
	PasswordFile string `toml:"password_file"`
	CAFile       string `toml:"ca_file"`
}

func (c InvitationSMTPConfig) Enabled() bool { return c != (InvitationSMTPConfig{}) }
func (c InvitationSMTPConfig) Validate() error {
	if c.Host == "" || len(c.Host) > 253 || c.Port < 1 || c.Port > 65535 || (c.TLSMode != "implicit" && c.TLSMode != "starttls") || (c.Username == "") != (c.PasswordFile == "") || len(c.Username) > 320 || strings.ContainsAny(c.Username, "\x00\r\n") {
		return access.ErrInvalid
	}
	if net.ParseIP(c.Host) == nil {
		for _, label := range strings.Split(c.Host, ".") {
			if label == "" || len(label) > 63 || strings.HasPrefix(label, "-") || strings.HasSuffix(label, "-") {
				return access.ErrInvalid
			}
			for _, r := range label {
				if !((r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') || r == '-') {
					return access.ErrInvalid
				}
			}
		}
	}
	if _, e := normalizeInvitationEmail(c.From); e != nil {
		return e
	}
	return nil
}

// This is the native Core EmailAddress policy. Controls are rejected before
// trimming; dots/plus tags are preserved and the complete bare address is lowercased.
func normalizeInvitationEmail(input string) (string, error) {
	for _, c := range input {
		if c > 127 || c < 32 || c == 127 {
			return "", access.ErrInvalid
		}
	}
	input = strings.Trim(input, " ")
	local, domain, ok := strings.Cut(input, "@")
	if !ok || len(input) > 254 || local == "" || len(local) > 64 || strings.HasPrefix(local, ".") || strings.HasSuffix(local, ".") || strings.Contains(local, "..") || domain == "" {
		return "", access.ErrInvalid
	}
	for _, c := range local {
		if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || strings.ContainsRune(".!#$%&'*+-/=?^_`{|}~", c)) {
			return "", access.ErrInvalid
		}
	}
	for _, label := range strings.Split(domain, ".") {
		if label == "" || len(label) > 63 || strings.HasPrefix(label, "-") || strings.HasSuffix(label, "-") {
			return "", access.ErrInvalid
		}
		for _, c := range label {
			if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-') {
				return "", access.ErrInvalid
			}
		}
	}
	return strings.ToLower(input), nil
}

type InvitationDelivery struct {
	State    string `json:"state"`
	SMTPCode int    `json:"smtp_code"`
}
type InvitationSender interface {
	SendInvitation(context.Context, string, string, string) InvitationDelivery
}
type InvitationSMTP struct {
	config        InvitationSMTPConfig
	password      string
	tls           *tls.Config
	origin        string
	messageDomain string
}

func NewInvitationSMTP(c InvitationSMTPConfig, password string, ca []byte, origin string) (*InvitationSMTP, error) {
	if c.Validate() != nil || (c.Username != "" && (password == "" || len(password) > 4096)) || (c.Username == "" && password != "") {
		return nil, access.ErrInvalid
	}
	u, e := url.Parse(origin)
	if e != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") || strings.ContainsAny(origin, "\x00\r\n") {
		return nil, access.ErrInvalid
	}
	tlsConfig := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: c.Host}
	if c.CAFile != "" {
		roots, e := x509.SystemCertPool()
		if e != nil {
			roots = x509.NewCertPool()
		}
		if !roots.AppendCertsFromPEM(ca) {
			return nil, access.ErrInvalid
		}
		tlsConfig.RootCAs = roots
	} else if len(ca) != 0 {
		return nil, access.ErrInvalid
	}
	c.From, _ = normalizeInvitationEmail(c.From)
	return &InvitationSMTP{config: c, password: password, tls: tlsConfig, origin: strings.TrimSuffix(origin, "/"), messageDomain: u.Hostname()}, nil
}
func invitationSMTPFailure(err error) InvitationDelivery {
	var p *textproto.Error
	if errors.As(err, &p) && p.Code >= 400 && p.Code < 600 {
		return InvitationDelivery{"rejected", p.Code}
	}
	return InvitationDelivery{"unknown", 0}
}
func (s *InvitationSMTP) SendInvitation(parent context.Context, operation, recipient, code string) InvitationDelivery {
	normalized, e := normalizeInvitationEmail(recipient)
	if e != nil || normalized != recipient || !invitationfacts.ValidCode(code) || len(operation) < 16 || len(operation) > 128 {
		return InvitationDelivery{"not_dispatched", 0}
	}
	ctx, cancel := context.WithTimeout(parent, 10*time.Second)
	defer cancel()
	conn, e := (&net.Dialer{}).DialContext(ctx, "tcp", net.JoinHostPort(s.config.Host, strconv.Itoa(s.config.Port)))
	if e != nil {
		return invitationSMTPFailure(e)
	}
	defer conn.Close()
	deadline, _ := ctx.Deadline()
	if e = conn.SetDeadline(deadline); e != nil {
		return invitationSMTPFailure(e)
	}
	stop := context.AfterFunc(ctx, func() { _ = conn.Close() })
	defer stop()
	var wire net.Conn = conn
	if s.config.TLSMode == "implicit" {
		secured := tls.Client(conn, s.tls.Clone())
		if e = secured.HandshakeContext(ctx); e != nil {
			return invitationSMTPFailure(e)
		}
		wire = secured
	}
	client, e := smtp.NewClient(wire, s.config.Host)
	if e != nil {
		return invitationSMTPFailure(e)
	}
	defer client.Close()
	if s.config.TLSMode == "starttls" {
		if e = client.StartTLS(s.tls.Clone()); e != nil {
			return invitationSMTPFailure(e)
		}
	}
	if s.config.Username != "" {
		if e = client.Auth(smtp.PlainAuth("", s.config.Username, s.password, s.config.Host)); e != nil {
			return invitationSMTPFailure(e)
		}
	}
	if e = client.Mail(s.config.From); e != nil {
		return invitationSMTPFailure(e)
	}
	if e = client.Rcpt(recipient); e != nil {
		return invitationSMTPFailure(e)
	}
	writer, e := client.Data()
	if e != nil {
		return invitationSMTPFailure(e)
	}
	link := s.origin + "/sign-up?aff=" + url.QueryEscape(code)
	message := fmt.Sprintf("From: %s\r\nTo: %s\r\nDate: %s\r\nSubject: An invitation to LMM\r\nMessage-ID: <%s@%s>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nAn LMM user invited you to join.\r\n\r\nRegister using this invitation link:\r\n%s\r\n\r\nThis invitation does not promise a monetary reward.\r\n", s.config.From, recipient, time.Now().UTC().Format(time.RFC1123Z), digest([]byte("invitation-mail:"+operation)), s.messageDomain, link)
	if _, e = writer.Write([]byte(message)); e != nil {
		return invitationSMTPFailure(e)
	}
	if e = writer.Close(); e != nil {
		return invitationSMTPFailure(e)
	}
	// Only 250 after DATA is acceptance. No delivery claim or automatic retry.
	return InvitationDelivery{"accepted_by_smtp", 250}
}
