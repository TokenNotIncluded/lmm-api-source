// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package app

import "testing"

func TestWalletPresetDirectory(t *testing.T) {
	for _, tc := range []struct{ state, plugins, providers, want string }{
		{"/var/lib/lmm/extensions", "", "/etc/lmm.conf.d/payments.toml", "/var/lib/lmm/extensions"},
		{"", "/srv/lmm/state/plugins.json", "/etc/lmm.conf.d/payments.toml", "/srv/lmm/state"},
		{"", "", "/srv/lmm/private/providers.toml", "/srv/lmm/private"},
		{"", "", "", ""},
	} {
		if got := walletPresetDirectory(tc.state, tc.plugins, tc.providers); got != tc.want {
			t.Fatalf("wallet preset directory: got %q, want %q", got, tc.want)
		}
	}
}
