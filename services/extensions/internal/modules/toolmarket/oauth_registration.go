// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
)

func (m *Module) registerOAuthClient(ctx context.Context, meta OAuthMeta) (string, error) {
	if meta.Registration == "" {
		return "", ErrAuth
	}
	body, err := json.Marshal(map[string]any{
		"client_name":                "LMM Tool Market",
		"redirect_uris":              []string{m.callback},
		"grant_types":                []string{"authorization_code", "refresh_token"},
		"response_types":             []string{"code"},
		"token_endpoint_auth_method": "none",
	})
	if err != nil {
		return "", err
	}
	resp, err := m.net.request(ctx, http.MethodPost, meta.Registration, body, http.Header{"Content-Type": {"application/json"}, "Accept": {"application/json"}})
	if err != nil {
		return "", err
	}
	data, err := readBodyLimit(resp, 32768)
	var registration struct {
		ClientID   string   `json:"client_id"`
		AuthMethod string   `json:"token_endpoint_auth_method"`
		Redirects  []string `json:"redirect_uris"`
	}
	if err != nil || resp.StatusCode != http.StatusCreated || uniqueJSON(data) != nil || json.Unmarshal(data, &registration) != nil {
		return "", ErrAuth
	}
	if registration.ClientID == "" || len(registration.ClientID) > 512 || strings.ContainsAny(registration.ClientID, "\r\n\x00") || registration.AuthMethod != "" && registration.AuthMethod != "none" || !contains(registration.Redirects, m.callback) {
		return "", ErrAuth
	}
	return registration.ClientID, nil
}

type oauthProviderInput struct {
	Provider string `json:"provider"`
}

var oauthPresets = []struct{ id, endpoint string }{
	{"agentkey", "https://api.agentkey.app/v1/mcp"},
	{"treg", "https://treg.to/mcp/"},
	{"treg-catalog", "https://treg.to/mcp/v2/"},
}

type oauthProviderStatus struct {
	FixedPriceTools []string `json:"fixed_price_tools"`
	Provider        string   `json:"provider"`
	Connected       bool     `json:"connected"`
	Installation    string   `json:"installation,omitempty"`
	Tools           []string `json:"tools"`
}

func (m *Module) oauthProviders(p Principal) ([]oauthProviderStatus, error) {
	rows := []oauthProviderStatus{}
	err := m.store.view(func(d database) error {
		for _, pair := range oauthPresets {
			id := digest([]any{"oauth-preset-installation", p, pair.id})
			status := oauthProviderStatus{Provider: pair.id, Tools: []string{}, FixedPriceTools: []string{}}
			if i, ok := d.Installs[id]; ok && i.Owner == p && i.Enabled {
				status.Installation = id
				secret := d.Secrets[id]
				status.Connected = !secret.Refreshing && secret.Access != "" && secret.Expires > m.now().Unix()
				if status.Connected {
					for _, tool := range d.Servers[i.Server].Tools {
						status.Tools = append(status.Tools, tool.Name)
						if ValidateLegacyProviderTool(pair.endpoint, tool.Name, nil) == nil {
							status.FixedPriceTools = append(status.FixedPriceTools, tool.Name)
						}
					}
				}
			}
			rows = append(rows, status)
		}
		return nil
	})
	return rows, err
}

func (m *Module) oauthPresetInstallation(p Principal, provider string) (string, error) {
	var endpoint, scopes string
	for _, pair := range oauthPresets {
		if pair.id == provider {
			endpoint = pair.endpoint
			break
		}
	}
	if endpoint == "" {
		return "", ErrInvalid
	}
	switch provider {
	case "treg":
		scopes = "treg:catalog treg:call treg:read"
	case "treg-catalog":
		scopes = "treg:catalog treg:call treg:read treg:directory"
	}
	serverID := digest([]any{"oauth-preset-server", p, provider})
	id := digest([]any{"oauth-preset-installation", p, provider})
	err := m.store.update(func(d *database) error {
		if i, ok := d.Installs[id]; ok {
			if i.Owner != p || i.Server != serverID || i.Release != "" || !i.Enabled {
				return ErrConflict
			}
			return nil
		}
		if len(d.Servers) >= 10000 || len(d.Installs) >= 20000 {
			return ErrLimit
		}
		d.Servers[serverID] = Server{ID: serverID, Owner: p, Endpoint: endpoint, Auth: "oauth", Scopes: scopes, Revision: 1, Tools: []Tool{}}
		d.Installs[id] = Installation{ID: id, Owner: p, Server: serverID, Enabled: true, Generation: 1, Loaded: map[string]string{}}
		d.audit(p, "register", serverID, "ok")
		return nil
	})
	return id, err
}
