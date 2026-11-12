package composition

import (
	"os"
	"path/filepath"
	"testing"
)

func TestPancakeConfigurationReplacement(t *testing.T) {
	path := filepath.Join(t.TempDir(), "payments.toml")
	old := []byte("version = 2\n")
	next := []byte("version = 2\n# operator update\n")
	if err := os.WriteFile(path, old, 0600); err != nil {
		t.Fatal(err)
	}
	if err := replacePancakeConfiguration(path, []byte("stale"), next); err == nil {
		t.Fatal("stale configuration accepted")
	}
	if err := replacePancakeConfiguration(path, old, next); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(path)
	if err != nil || string(raw) != string(next) {
		t.Fatal("replacement not committed", err)
	}
	info, err := os.Stat(path)
	if err != nil || info.Mode().Perm() != 0600 {
		t.Fatal("private mode lost", err)
	}
	files, err := filepath.Glob(filepath.Join(filepath.Dir(path), ".pancake-config-*"))
	if err != nil || len(files) != 0 {
		t.Fatal("staging files leaked", err)
	}
}
