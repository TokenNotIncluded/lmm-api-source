package ssmcpd

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"path/filepath"
	"regexp"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
)

type InternalConfig struct {
	Origin    string `toml:"origin"`
	TokenFile string `toml:"token_file"`
}

type internalClient struct {
	http   *http.Client
	origin string
	token  string
}

var bearerPattern = regexp.MustCompile("^[0-9a-f]{64}$")

func newInternalClient(cfg InternalConfig) (*internalClient, error) {
	u, err := url.Parse(cfg.Origin)
	if err != nil || u.Hostname() == "" || u.User != nil || u.Path != "" || cfg.Origin != u.Scheme+"://"+u.Host || !(u.Scheme == "https" || u.Scheme == "http" && net.ParseIP(u.Hostname()).IsLoopback()) {
		return nil, errors.New("ssmcpd internal origin must use HTTPS or loopback HTTP and contain no credentials or path")
	}
	if !filepath.IsAbs(cfg.TokenFile) || filepath.Clean(cfg.TokenFile) != cfg.TokenFile {
		return nil, errors.New("ssmcpd requires an absolute private credential path")
	}
	token, err := coreclient.ReadTokenFile(cfg.TokenFile)
	if err != nil {
		return nil, err
	}
	if !bearerPattern.Match(token) {
		return nil, errors.New("invalid ssmcpd internal service credential")
	}
	dialer := net.Dialer{Timeout: 2 * time.Second}
	transport := &http.Transport{DialContext: dialer.DialContext, DisableCompression: true, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, MaxResponseHeaderBytes: 16 << 10, TLSHandshakeTimeout: 2 * time.Second}
	return &internalClient{http: &http.Client{Transport: transport, Timeout: 4 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, origin: cfg.Origin, token: string(token)}, nil
}

func (c *internalClient) post(ctx context.Context, path string, payload any) (int, []byte, error) {
	if path != "/internal/v1/ssmcp/quota" && path != "/internal/v1/ssmcp/entitlements" {
		return 0, nil, errors.New("unsupported ssmcpd endpoint")
	}
	data, err := json.Marshal(payload)
	if err != nil {
		return 0, nil, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.origin+path, bytes.NewReader(data))
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	res, err := c.http.Do(req)
	if err != nil {
		return 0, nil, errors.New("ssmcpd unavailable")
	}
	defer res.Body.Close()
	const maximum = 16 << 10
	body, err := io.ReadAll(io.LimitReader(res.Body, maximum+1))
	if err != nil || len(body) > maximum {
		return 0, nil, errors.New("invalid ssmcpd reply")
	}
	return res.StatusCode, body, nil
}
