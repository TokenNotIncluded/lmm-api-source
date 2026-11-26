// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "testing"

func TestMarketPresetCredentialIsPerCallerAndEndpoint(t *testing.T) {
	d := emptyDB()
	preset := oauthPresets[0]
	publisher := Principal{User: "1", Account: "1"}
	buyer := Principal{User: "2", Account: "2"}
	install := func(p Principal, token string) (string, string) {
		sid := digest([]any{"oauth-preset-server", p, preset.id})
		iid := digest([]any{"oauth-preset-installation", p, preset.id})
		d.Servers[sid] = Server{ID: sid, Owner: p, Auth: "oauth", Endpoint: preset.endpoint}
		d.Installs[iid] = Installation{ID: iid, Server: sid, Owner: p, Enabled: true, Generation: 1}
		d.Secrets[iid] = Secret{Access: token, Expires: 200}
		return sid, iid
	}
	sid, _ := install(publisher, "provider-publisher")
	service := WebServiceRecord{ID: "s", OwnerID: 1}
	version := WebVersion{ServiceID: "s", ExecutionType: "remote", Endpoint: preset.endpoint, OAuthServerID: sid}
	if _, err := marketPresetCredential(d, buyer, service, version, 100); err == nil {
		t.Fatal("used publisher credentials for buyer")
	}
	_, iid := install(buyer, "provider-buyer")
	got, err := marketPresetCredential(d, buyer, service, version, 100)
	if err != nil || got.Secret != "provider-buyer" || got.Mode != "oauth" {
		t.Fatal("caller credential not selected", err)
	}
	if _, err := marketPresetCredential(d, Principal{User: "2", Account: "3"}, service, version, 100); err == nil {
		t.Fatal("accepted wrong account")
	}
	if _, err := marketPresetCredential(d, buyer, service, version, 200); err == nil {
		t.Fatal("accepted expired credential")
	}
	bad := version
	bad.Endpoint = "https://different.example/mcp"
	if _, err := marketPresetCredential(d, buyer, service, bad, 100); err == nil {
		t.Fatal("accepted different recipient")
	}
	delete(d.Secrets, iid)
	if _, err := marketPresetCredential(d, buyer, service, version, 100); err == nil {
		t.Fatal("used publisher after buyer revoked")
	}
}
