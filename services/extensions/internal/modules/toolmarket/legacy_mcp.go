// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strconv"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type LegacyMarketMCPOptions struct {
	LocalHTTP bool
	// Both values come from trusted host configuration, never request Host.
	Issuer              string
	Resource            string
	ResourceMetadataURL string
}

func legacyMCPOptionsValid(opts LegacyMarketMCPOptions) bool {
	resource, e := url.Parse(opts.Resource)
	if e != nil || resource.Path != "/mcp/market" {
		return false
	}
	var same netip.Addr
	for _, raw := range []string{opts.Issuer, opts.Resource, opts.ResourceMetadataURL} {
		if !legacyAbsoluteResource(raw) {
			return false
		}
		u, _ := url.Parse(raw)
		if !opts.LocalHTTP {
			if u.Scheme != "https" {
				return false
			}
			continue
		}
		if u.Scheme != "http" {
			return false
		}
		ip, e := netip.ParseAddr(u.Hostname())
		if e != nil || !ip.IsLoopback() || ip.Zone() != "" {
			return false
		}
		ip = ip.Unmap()
		port, e := strconv.Atoi(u.Port())
		if e != nil || port < 1024 || port > 65535 {
			return false
		}
		if same.IsValid() && ip != same {
			return false
		}
		same = ip
	}
	return true
}
func legacyMCPChallenge(opts LegacyMarketMCPOptions, scopes []string, code string) string {
	parts := []string{"Bearer resource_metadata=" + strconv.Quote(opts.ResourceMetadataURL)}
	if len(scopes) > 0 {
		parts = append(parts, "scope="+strconv.Quote(strings.Join(scopes, " ")))
	}
	if code != "" {
		parts = append(parts, "error="+strconv.Quote(code))
	}
	return strings.Join(parts, ", ")
}

const marketOAuthHint = "Connect this MCP client with OAuth for /mcp/market using resource_metadata in WWW-Authenticate."

func legacyMCPRequired(raw []byte) (invoke, manage bool) {
	var calls []json.RawMessage
	if bytes.HasPrefix(bytes.TrimSpace(raw), []byte("[")) {
		_ = json.Unmarshal(raw, &calls)
	} else {
		calls = []json.RawMessage{raw}
	}
	for _, row := range calls {
		var msg struct {
			Method string
			Params struct {
				Name      string
				Arguments json.RawMessage
			}
		}
		if json.Unmarshal(row, &msg) != nil || msg.Method != "tools/call" || msg.Params.Name != "metamcp" {
			continue
		}
		steps, err := parseMarketCode(msg.Params.Arguments)
		if err != nil {
			continue
		}
		for _, step := range steps {
			i, m := marketActionScopes(step.action)
			invoke = invoke || i
			manage = manage || m
		}
	}
	return
}
func (m *Module) LegacyMarketMCPHandler(opts LegacyMarketMCPOptions) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "POST, GET, DELETE, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Authorization, Content-Type, Accept, Mcp-Protocol-Version, Mcp-Session-Id")
		w.Header().Set("Access-Control-Expose-Headers", "WWW-Authenticate, Mcp-Session-Id")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		if r.URL.Path != "/mcp/market" && r.URL.Path != "/mcp/market/" {
			http.NotFound(w, r)
			return
		}
		if !legacyMCPOptionsValid(opts) {
			http.Error(w, "MCP configuration unavailable", 503)
			return
		}
		if !m.legacyMCPAcknowledged() {
			http.Error(w, "MCP Core capability acknowledgement unavailable", 503)
			return
		}
		values := r.Header.Values("Authorization")
		raw := ""
		if len(values) == 1 && len(values[0]) > 7 && strings.EqualFold(values[0][:7], "Bearer ") {
			raw = values[0][7:]
		}
		var p LegacyMarketPrincipal
		e := ErrDenied
		if strings.HasPrefix(raw, "lmm_at_") {
			p, e = m.LegacyAuthenticateMarket(r.Context(), raw, []string{"market:discover"})
		}
		if e != nil {
			status, code := 401, "invalid_token"
			if raw == "" {
				code = ""
			}
			if errors.Is(e, ErrLegacyMarketScope) {
				status, code = 403, "insufficient_scope"
			}
			if errors.Is(e, ErrUnavailable) {
				http.Error(w, "MCP authority unavailable", 503)
				return
			}
			w.Header().Set("WWW-Authenticate", legacyMCPChallenge(opts, []string{"market:discover"}, code))
			message := marketOAuthHint
			if status == http.StatusForbidden {
				message = "Authorize market:discover to access the MCP tool market."
			}
			http.Error(w, message, status)
			return
		}
		if p.CredentialKind != "oauth" || p.Issuer != opts.Issuer || p.Resource != opts.Resource {
			w.Header().Set("WWW-Authenticate", legacyMCPChallenge(opts, nil, "invalid_token"))
			http.Error(w, marketOAuthHint, http.StatusUnauthorized)
			return
		}
		q, qe := url.ParseQuery(r.URL.RawQuery)
		mode := q.Get("mode")
		alternate := false
		for _, header := range []string{"X-Account-ID", "X-User-ID", "X-Api-Key", "Api-Key", "X-Goog-Api-Key", "Mj-Api-Secret", "Sec-Websocket-Protocol", "New-Api-User", "Content-Encoding", "X-LMM-Group", "X-OAuth-Group", "X-Lmm-User-Credential", "Cookie"} {
			if len(r.Header.Values(header)) > 0 {
				alternate = true
			}
		}
		if qe != nil || len(q) > 1 || len(q) == 1 && len(q["mode"]) != 1 || mode != "" && mode != "full" && mode != "compact" || alternate {
			http.Error(w, "Send one OAuth bearer credential for /mcp/market. Remove additional credentials, cookies and unsupported query parameters.", 400)
			return
		}
		r.Body = http.MaxBytesReader(w, r.Body, 256<<10)
		if r.Method == http.MethodPost {
			body, readErr := io.ReadAll(r.Body)
			if readErr != nil {
				http.Error(w, "MCP request too large", 413)
				return
			}
			r.Body = io.NopCloser(bytes.NewReader(body))
			invoke, manage := legacyMCPRequired(body)
			if invoke && !p.CanInvoke || manage && !p.CanManage {
				scopes := []string{"market:discover"}
				if invoke || p.CanInvoke {
					scopes = append(scopes, "market:invoke")
				}
				if manage || p.CanManage {
					scopes = append(scopes, "market:manage")
				}
				w.Header().Set("WWW-Authenticate", legacyMCPChallenge(opts, scopes, "insufficient_scope"))
				http.Error(w, "MCP connection needs additional permissions", 403)
				return
			}
		}
		ctx := r.Context()
		if host, _, err := net.SplitHostPort(r.RemoteAddr); err == nil {
			ctx = WithBuiltinPeerIP(ctx, host)
		}
		r = r.WithContext(ctx)
		server := mcp.NewServer(&mcp.Implementation{Name: "lmm-tool-market", Version: "3"}, nil)
		server.AddTool(LegacyMetaToolDescription(), func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			return m.runMarketCode(ctx, raw, p, req)
		})
		handler := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return server }, &mcp.StreamableHTTPOptions{Stateless: true, JSONResponse: true, DisableLocalhostProtection: true, PropagateRequestCancellation: true})
		handler.ServeHTTP(w, r)
	})
}
