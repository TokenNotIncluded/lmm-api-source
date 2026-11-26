// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"net/url"
	"testing"
)

func TestDynamicProviderRegistrationAndReauthorization(t *testing.T) {
	mock := newMockConfigured(t, func(m *mockMCP) { m.registration = true })
	m := testModule(t, mock)
	out, err := m.register(alice, RegisterInput{Endpoint: mock.server.URL + "/mcp", Auth: "oauth"})
	if err != nil {
		t.Fatal(err)
	}
	id := out.(map[string]any)["installation"].(Installation).ID
	start := doOAuth(t, m, alice, id)
	auth, err := url.Parse(start.URL)
	if err != nil || auth.Query().Get("client_id") != "client" || len(auth.Query()["resource"]) != 1 || auth.Query().Get("resource") != mock.server.URL+"/mcp" || auth.Query().Get("code_challenge_method") != "S256" {
		t.Fatalf("invalid authorization request")
	}
	if err = m.refreshOAuth(context.Background(), alice, id); err != nil {
		t.Fatal(err)
	}
	if _, err = m.revoke(context.Background(), alice, id); err != nil {
		t.Fatal(err)
	}
	doOAuth(t, m, alice, id)
	if mock.registrations.Load() != 1 {
		t.Fatal("public registration was not reused")
	}
}
func TestProviderQuickConnectReusesOnlyOwnerInstallation(t *testing.T) {
	m := testModule(t, newMock(t))
	id, err := m.oauthPresetInstallation(alice, "treg")
	if err != nil {
		t.Fatal(err)
	}
	repeat, err := m.oauthPresetInstallation(alice, "treg")
	if err != nil || id != repeat {
		t.Fatal("duplicate installation")
	}
	other, err := m.oauthPresetInstallation(bob, "treg")
	if err != nil || other == id {
		t.Fatal("owner isolation")
	}
	catalog, err := m.oauthPresetInstallation(alice, "treg-catalog")
	if err != nil || catalog == id {
		t.Fatal("catalog audience isolation")
	}
	if _, err = m.oauthPresetInstallation(alice, "https://example.org"); err == nil {
		t.Fatal("unknown preset accepted")
	}
}

func TestOAuthToolImportPreservesPublisherAndChosenPrice(t *testing.T) {
	m := testModule(t, newMock(t))
	user := Principal{User: "17", Account: "17"}
	id, err := m.oauthPresetInstallation(user, "agentkey")
	if err != nil {
		t.Fatal(err)
	}
	if err = m.store.update(func(d *database) error {
		i := d.Installs[id]
		s := d.Servers[i.Server]
		s.Tools = []Tool{{Name: "list_tools", Description: "Lookup", InputSchema: json.RawMessage(`{"type":"object","properties":{"q":{"type":"string"}}}`)}}
		d.Servers[s.ID] = s
		d.Secrets[id] = Secret{Access: "fake-user-token", Expires: m.now().Unix() + 3600}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	imported, err := m.importOAuthTool(user, oauthImportInput{Provider: "agentkey", Name: "list_tools", PriceQuota: 321})
	if err != nil {
		t.Fatal(err)
	}
	again, err := m.importOAuthTool(user, oauthImportInput{Provider: "agentkey", Name: "list_tools", PriceQuota: 800})
	if err != nil || again.DraftVersionID != imported.DraftVersionID {
		t.Fatal("repeat import overwrote draft", err)
	}
	err = m.store.view(func(d database) error {
		ver := d.Web.Versions[imported.DraftVersionID]
		if ver.Version.OAuthServerID != d.Installs[id].Server || ver.Version.Visibility != "private" || ver.Version.Status != "draft" || ver.Tools[0].PriceQuota != 321 || imported.LiveVersionID != "" {
			t.Fatal("imported draft has incorrect pricing or visibility")
		}
		cred, e := m.webOwnerCredential(d, imported, ver.Version)
		if e != nil || cred.Secret != "fake-user-token" {
			t.Fatal("source credential mismatch", e)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = m.importOAuthTool(Principal{User: "18", Account: "18"}, oauthImportInput{Provider: "agentkey", Name: "list_tools", PriceQuota: 1}); err == nil {
		t.Fatal("cross-owner import accepted")
	}
}
