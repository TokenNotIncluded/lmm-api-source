// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import "strconv"

// A reviewed provider version identifies a trusted preset. Each caller brings
// their own OAuth installation; the publisher's token is never a fallback.
func marketPresetCredential(d database, actor Principal, service WebServiceRecord, version WebVersion, now int64) (webCredential, error) {
	source, ok := d.Servers[version.OAuthServerID]
	if !ok || service.ID != version.ServiceID || source.ID != version.OAuthServerID || source.Auth != "oauth" || source.Endpoint != version.Endpoint || source.Owner.User != strconv.FormatInt(service.OwnerID, 10) || version.ExecutionType != "remote" {
		return webCredential{}, ErrConflict
	}
	for _, preset := range oauthPresets {
		if preset.endpoint != version.Endpoint {
			continue
		}
		if source.ID != digest([]any{"oauth-preset-server", source.Owner, preset.id}) {
			return webCredential{}, ErrConflict
		}
		iid := digest([]any{"oauth-preset-installation", actor, preset.id})
		install, release, secret, err := selected(d, actor, iid)
		if err != nil {
			return webCredential{}, err
		}
		if install.ID != iid || install.Server != digest([]any{"oauth-preset-server", actor, preset.id}) || release.Server.ID != install.Server || release.Server.Owner != actor || release.Server.Endpoint != version.Endpoint || release.Server.Auth != "oauth" {
			return webCredential{}, ErrConflict
		}
		if secret.Access == "" || secret.Expires <= now || secret.Refreshing {
			return webCredential{}, ErrAuth
		}
		return webCredential{Mode: "oauth", Secret: secret.Access, oauthGeneration: install.Generation, oauthExpires: secret.Expires}, nil
	}
	return webCredential{}, ErrConflict
}
