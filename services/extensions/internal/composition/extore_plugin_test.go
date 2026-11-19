package composition

import (
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

func TestExtorePluginRequiresStoreSelection(t *testing.T) {
	cfg := Config{
		Selected: []string{"store"}, Store: StorageConfig{Database: "store", URLFile: "/tmp/store.url"},
		StoreExtoreKeyFile: "/tmp/extore.key", StoreExtoreRedirectURI: "https://shop.example/store/manage",
		StoreExtorePlugin: extore.PluginConfig{Origin: "https://extore.example", Socket: "/run/lmm/extore/catalog.sock",
			Host: "/usr/libexec/lmm-extore-host", Library: "/usr/lib/lmm/libextore_zplugin.so",
			SHA256: strings.Repeat("a", 64), DataDir: "/var/lib/extore"},
	}
	if err := validateExtorePluginSelection(cfg); err != nil {
		t.Fatal(err)
	}
	cfg.Selected = nil
	if err := validateExtorePluginSelection(cfg); err == nil {
		t.Fatal("plugin accepted without store")
	}
	cfg.Selected = []string{"store"}
	cfg.StoreExtoreKeyFile = ""
	if err := validateExtorePluginSelection(cfg); err == nil {
		t.Fatal("plugin accepted without Extore authorization")
	}
}
