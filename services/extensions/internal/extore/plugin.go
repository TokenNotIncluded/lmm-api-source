package extore

import (
	"context"
	"encoding/hex"
	"errors"
	"net"
	"net/http"
	"path/filepath"
	"strings"
)

// PluginConfig is operator-owned configuration. No browser request selects a
// native library, state directory, socket, or the origin bound to that instance.
type PluginConfig struct {
	Origin  string `toml:"origin"`
	Socket  string `toml:"socket"`
	Host    string `toml:"host"`
	Library string `toml:"library"`
	SHA256  string `toml:"sha256"`
	DataDir string `toml:"data_dir"`
}

func (c PluginConfig) Enabled() bool {
	return c != (PluginConfig{})
}

func (c PluginConfig) Validate() error {
	if !c.Enabled() {
		return nil
	}
	origin, err := NormalizeOrigin(c.Origin)
	if err != nil || c.Origin == "" || origin != c.Origin {
		return errors.New("store Extore plugin requires a canonical HTTPS origin")
	}
	for _, path := range []string{c.Socket, c.Host, c.Library, c.DataDir} {
		if !filepath.IsAbs(path) || filepath.Clean(path) != path || strings.ContainsRune(path, 0) {
			return errors.New("store Extore plugin requires absolute host, library, data_dir and socket paths")
		}
	}
	digest, err := hex.DecodeString(c.SHA256)
	if err != nil || len(digest) != 32 || strings.ToLower(c.SHA256) != c.SHA256 {
		return errors.New("store Extore plugin requires the reviewed library SHA256")
	}
	return nil
}

// PluginHost owns the private worker and the catalogue-only Unix transport.
// OAuth discovery, approval, exchange, refresh and revocation remain on HTTP.
type PluginHost struct {
	origin    string
	transport *http.Transport
	close     func() error
}

func (p *PluginHost) Close() error {
	if p == nil || p.close == nil {
		return nil
	}
	return p.close()
}

func (p *PluginHost) WrapHTTP(base *http.Client) (*http.Client, error) {
	if p == nil || p.origin == "" || p.transport == nil || base == nil {
		return nil, ErrInput
	}
	remote := base.Transport
	if remote == nil {
		remote = http.DefaultTransport
	}
	client := *base
	client.Transport = pluginCatalogTransport{origin: p.origin, local: p.transport, remote: remote}
	return &client, nil
}

type pluginCatalogTransport struct {
	origin string
	local  http.RoundTripper
	remote http.RoundTripper
}

func (p pluginCatalogTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	u := request.URL
	if request.Method != http.MethodGet || u.User != nil ||
		u.Scheme+"://"+u.Host != p.origin || u.EscapedPath() != catalogPath ||
		u.RawQuery != "" || u.ForceQuery {
		return p.remote.RoundTrip(request)
	}
	local := request.Clone(request.Context())
	endpoint := *u
	endpoint.Scheme, endpoint.Host = "http", "extore.plugin"
	local.URL, local.Host = &endpoint, endpoint.Host
	local.Header.Set("X-Extore-Origin", p.origin)
	// An unavailable configured plugin must not silently disclose its token to
	// another transport or use a different source of product authority.
	return p.local.RoundTrip(local)
}

func pluginTransport(socket string) *http.Transport {
	return &http.Transport{
		DisableKeepAlives: true,
		MaxConnsPerHost:   1,
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			if err := checkPluginSocket(socket); err != nil {
				return nil, err
			}
			return (&net.Dialer{}).DialContext(ctx, "unix", socket)
		},
	}
}
