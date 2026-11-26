// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "strconv"

type oauthImportInput struct {
	Provider   string `json:"provider"`
	Name       string `json:"name"`
	PriceQuota int64  `json:"price_quota"`
}

func (m *Module) importOAuthTool(p Principal, in oauthImportInput) (WebServiceRecord, error) {
	user, err := strconv.ParseInt(p.User, 10, 64)
	if err != nil || user <= 0 || p.User != p.Account {
		return WebServiceRecord{}, ErrDenied
	}
	id := digest([]any{"oauth-preset-installation", p, in.Provider})
	inst, release, secret, err := m.snapshot(p, id)
	if err != nil {
		return WebServiceRecord{}, err
	}
	server := release.Server
	if inst.Release != "" || server.Owner != p || server.Auth != "oauth" || secret.Access == "" || secret.Refreshing || secret.Expires <= m.now().Unix() && secret.Refresh == "" {
		return WebServiceRecord{}, ErrAuth
	}
	for _, t := range server.Tools {
		if t.Name != in.Name {
			continue
		}
		input := WebDraftInput{Name: in.Provider + " / " + t.Name, Description: t.Description, ExecutionType: "remote", Visibility: "private", Endpoint: server.Endpoint, Tools: []WebToolInput{{Name: t.Name, Description: t.Description, InputSchema: t.InputSchema, OutputSchema: t.OutputSchema, Permissions: []string{"network", "external_account"}, PriceQuota: in.PriceQuota}}}
		return m.saveWebDraft(p, user, digest([]any{"oauth-preset-draft", p, in.Provider, t.Name}), input, server.ID)
	}
	return WebServiceRecord{}, ErrMissing
}

func (m *Module) webOwnerCredential(d database, service WebServiceRecord, v WebVersion) (webCredential, error) {
	if v.OAuthServerID == "" {
		return webResolveCredential(d, service.OwnerID, service.ID, v.ID, v.Endpoint)
	}
	owner := strconv.FormatInt(service.OwnerID, 10)
	return marketPresetCredential(d, Principal{User: owner, Account: owner}, service, v, m.now().Unix())
}
