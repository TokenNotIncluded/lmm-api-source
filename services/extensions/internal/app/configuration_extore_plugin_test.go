package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestExtorePluginTypedConfig(t *testing.T) {
	base := "version=1\n[host]\ntoken_file='/tmp/host-token'\nmodules=['store']\nweb_compat=true\n"
	store := "[store]\ndatabase_name='lmm_store_test'\ndatabase_url_file='/tmp/store.url'\nextore_key_file='/tmp/extore.key'\nextore_redirect_uri='https://shop.example/store/manage'\n"
	plugin := "[store.extore_plugin]\norigin='https://extore.example'\nsocket='/run/lmm/extore/catalog.sock'\nhost='/usr/libexec/lmm-extore-host'\nlibrary='/usr/lib/lmm/libextore_zplugin.so'\nsha256='" + strings.Repeat("a", 64) + "'\ndata_dir='/var/lib/extore'\n"
	write := func(raw string) string {
		path := filepath.Join(t.TempDir(), "extensions.toml")
		if err := os.WriteFile(path, []byte(raw), 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	cfg, err := LoadConfig(write(base + store + plugin))
	if err != nil || cfg.Store.ExtorePlugin.Origin != "https://extore.example" ||
		cfg.Store.ExtorePlugin.Library != "/usr/lib/lmm/libextore_zplugin.so" {
		t.Fatal("operator plugin binding lost", err)
	}
	for _, raw := range []string{
		base + plugin,
		base + store + strings.Replace(plugin, "origin=", "Origin=", 1),
		base + store + strings.Replace(plugin, "sha256=", "SHA256=", 1),
		base + store + plugin + "core_database='private-value'\n",
		base + store + strings.Replace(plugin, "https://extore.example", "https://other.example/path", 1),
		base + store + strings.Replace(plugin, "/var/lib/extore", "relative", 1),
	} {
		_, err := LoadConfig(write(raw))
		if err == nil {
			t.Fatal("accepted invalid plugin configuration")
		}
		if strings.Contains(err.Error(), "private-value") {
			t.Fatal("configuration value leaked")
		}
	}
}
