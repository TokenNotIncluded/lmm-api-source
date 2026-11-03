// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package app

import (
	"os"
	"path/filepath"
	"testing"
)

func TestCommunityConfiguration(t *testing.T) {
	dir := t.TempDir()
	write := func(body string) string {
		path := filepath.Join(dir, "extensions.toml")
		if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	base := "version = 1\n[host]\ntoken_file = \"/private/host\"\nmodules = [\"community\"]\n[core]\nsocket = \"/run/core.sock\"\nidentity_token_file = \"/private/identity\"\n"
	cfg, err := LoadConfig(write(base + "[community]\ndatabase_name = \"lmm_community\"\ndatabase_url_file = \"/private/community-url\"\n"))
	if err != nil || cfg.Community.DatabaseName != "lmm_community" {
		t.Fatalf("community section rejected: %v", err)
	}
	for _, bad := range []string{"[community]\ndatabase_name = \"lmm_community\"\n", "[community]\ndatabase_name = \"lmm_community\"\ndatabase_url_file = \"relative\"\n", "[community]\nextra = 1\n"} {
		if _, err := LoadConfig(write(base + bad)); err == nil {
			t.Fatalf("accepted %q", bad)
		}
	}
}
