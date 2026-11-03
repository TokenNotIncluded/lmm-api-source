// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package app

import (
	"os"
	"path/filepath"
	"testing"
)

func TestAcquisitionConfiguration(t *testing.T) {
	dir := t.TempDir()
	write := func(body string) string {
		path := filepath.Join(dir, "extensions.toml")
		if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	base := "version = 1\n[host]\ntoken_file = \"/private/host\"\nmodules = [\"acquisition\"]\n[core]\nsocket = \"/run/core.sock\"\nidentity_token_file = \"/private/identity\"\n"
	cfg, err := LoadConfig(write(base + "[acquisition]\ndatabase_name = \"lmm_acquisition\"\ndatabase_url_file = \"/private/acquisition-url\"\n"))
	if err != nil || cfg.Acquisition.DatabaseName != "lmm_acquisition" {
		t.Fatalf("acquisition section rejected: %v", err)
	}
	for _, bad := range []string{"[acquisition]\ndatabase_name = \"lmm_acquisition\"\n", "[acquisition]\ndatabase_name = \"lmm_acquisition\"\ndatabase_url_file = \"relative\"\n", "[acquisition]\nextra = 1\n"} {
		if _, err := LoadConfig(write(base + bad)); err == nil {
			t.Fatalf("accepted %q", bad)
		}
	}
}
