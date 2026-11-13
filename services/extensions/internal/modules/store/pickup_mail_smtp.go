// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

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
	"regexp"
	"strconv"
	"strings"
	"time"
)

// PickupSMTPConfig is a closed TLS-only SMTP configuration. Secrets are loaded
// by composition from private files, never request input or ambient SMTP env.
type PickupSMTPConfig struct {
	Host         string `toml:"smtp_host"`
	Port         int    `toml:"smtp_port"`
	TLSMode      string `toml:"tls_mode"`
	From         string `toml:"from"`
	Username     string `toml:"username"`
	PasswordFile string `toml:"password_file"`
	CAFile       string `toml:"ca_file"`
	Origin       string `toml:"origin"`
	KeyFile      string `toml:"key_file"`
}

func (c PickupSMTPConfig) Enabled() bool { return c != (PickupSMTPConfig{}) }
func (c PickupSMTPConfig) Validate() error {
	u, err := url.Parse(c.Origin)
	hostOK := net.ParseIP(c.Host) != nil || regexp.MustCompile(`^[a-zA-Z0-9](?:[a-zA-Z0-9.-]*[a-zA-Z0-9])?$`).MatchString(c.Host)
	if !hostOK || c.Host == "" || len(c.Host) > 253 || strings.ContainsAny(c.Host, "/\\ \t\r\n\x00") || c.Port < 1 || c.Port > 65535 || (c.TLSMode != "starttls" && c.TLSMode != "implicit") || !pickupEmailValid(c.From) || c.KeyFile == "" || err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") || (c.Username == "") != (c.PasswordFile == "") || strings.ContainsAny(c.Username, "\r\n\x00") {
		return ErrInvalid
	}
	return nil
}

type PickupSMTP struct {
	config   PickupSMTPConfig
	password string
	tls      *tls.Config
}

func NewPickupSMTP(c PickupSMTPConfig, password string, ca []byte) (*PickupSMTP, error) {
	if c.Validate() != nil || (c.Username != "" && (password == "" || len(password) > 4096)) {
		return nil, ErrInvalid
	}
	tlsConfig := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: c.Host}
	if c.CAFile != "" {
		roots, err := x509.SystemCertPool()
		if err != nil {
			roots = x509.NewCertPool()
		}
		if !roots.AppendCertsFromPEM(ca) {
			return nil, ErrInvalid
		}
		tlsConfig.RootCAs = roots
	}
	return &PickupSMTP{c, password, tlsConfig}, nil
}

type pickupSMTPResult struct {
	state string
	code  int
}

func smtpFailure(err error, uncertain bool) pickupSMTPResult {
	var protocol *textproto.Error
	if errors.As(err, &protocol) {
		if protocol.Code >= 500 && protocol.Code < 600 {
			return pickupSMTPResult{"rejected", protocol.Code}
		}
		if protocol.Code >= 400 && protocol.Code < 500 {
			return pickupSMTPResult{"retry_wait", protocol.Code}
		}
	}
	if uncertain {
		return pickupSMTPResult{"unknown", 0}
	}
	return pickupSMTPResult{"retry_wait", 0}
}
func (s *PickupSMTP) send(parent context.Context, id string, p pickupMailPayload) pickupSMTPResult {
	ctx, cancel := context.WithTimeout(parent, 10*time.Second)
	defer cancel()
	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", net.JoinHostPort(s.config.Host, strconv.Itoa(s.config.Port)))
	if err != nil {
		return smtpFailure(err, false)
	}
	defer conn.Close()
	deadline, _ := ctx.Deadline()
	if err = conn.SetDeadline(deadline); err != nil {
		return smtpFailure(err, false)
	}
	stop := context.AfterFunc(ctx, func() { _ = conn.Close() })
	defer stop()
	var wire net.Conn = conn
	if s.config.TLSMode == "implicit" {
		secured := tls.Client(conn, s.tls.Clone())
		if err = secured.HandshakeContext(ctx); err != nil {
			return smtpFailure(err, false)
		}
		wire = secured
	}
	client, err := smtp.NewClient(wire, s.config.Host)
	if err != nil {
		return smtpFailure(err, false)
	}
	defer client.Close()
	if s.config.TLSMode == "starttls" {
		if err = client.StartTLS(s.tls.Clone()); err != nil {
			return smtpFailure(err, false)
		}
	}
	if s.config.Username != "" {
		if err = client.Auth(smtp.PlainAuth("", s.config.Username, s.password, s.config.Host)); err != nil {
			return smtpFailure(err, false)
		}
	}
	if err = client.Mail(s.config.From); err != nil {
		return smtpFailure(err, false)
	}
	if err = client.Rcpt(p.Recipient); err != nil {
		return smtpFailure(err, false)
	}
	writer, err := client.Data()
	if err != nil {
		return smtpFailure(err, false)
	}
	// id is our stable SHA-256 command ID. No recipient, token or order contents
	// enter diagnostic errors. SMTP's mature library owns framing and dot-stuffing.
	message := fmt.Sprintf("From: %s\r\nTo: %s\r\nDate: %s\r\nSubject: Your store pickup link\r\nMessage-ID: <%s@lmm-store.invalid>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nYour private pickup link:\r\n%s\r\n", s.config.From, p.Recipient, time.Unix(p.CreatedAt, 0).UTC().Format(time.RFC1123Z), id, p.URL)
	if _, err = writer.Write([]byte(message)); err != nil {
		return smtpFailure(err, true)
	}
	if err = writer.Close(); err != nil {
		return smtpFailure(err, true)
	}
	// 250 after DATA is the acceptance boundary. QUIT failure cannot revoke it.
	return pickupSMTPResult{"accepted_by_smtp", 250}
}
