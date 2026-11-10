package composition

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"
)

func TestScriptRepositoryKeyIsolation(t *testing.T) {
	dir := t.TempDir()
	key := bytes.Repeat([]byte("k"), 32)
	path := filepath.Join(dir, "repository-key")
	otherPath := filepath.Join(dir, "stock-key")
	if err := os.WriteFile(path, key, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(otherPath, key, 0600); err != nil {
		t.Fatal(err)
	}
	cfg := Config{ScriptRepositoryKeyFile: path}
	if _, err := scriptRepositoryKey(cfg, key); err == nil {
		t.Fatal("host credential reused as encryption key")
	}
	cfg.StoreStockKeyFile = otherPath
	if _, err := scriptRepositoryKey(cfg); err == nil {
		t.Fatal("same key material under a different path accepted")
	}
	if err := os.WriteFile(otherPath, bytes.Repeat([]byte("s"), 32), 0600); err != nil {
		t.Fatal(err)
	}
	if got, err := scriptRepositoryKey(cfg, bytes.Repeat([]byte("h"), 32)); err != nil || !bytes.Equal(got, key) {
		t.Fatal("independent key rejected", err)
	}
	if err := os.WriteFile(path, key[:31], 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := scriptRepositoryKey(cfg); err == nil {
		t.Fatal("short encryption key accepted")
	}
}
