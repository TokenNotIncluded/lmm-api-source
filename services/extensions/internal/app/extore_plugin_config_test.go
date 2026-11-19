package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestExtoreNativePluginConfiguration(t *testing.T) {
	const base = "version=1\n[host]\ntoken_file='/tmp/host-token'\nmodules=['store']\nweb_compat=true\n" +
		"[store]\ndatabase_name='lmm_store_test'\ndatabase_url_file='/tmp/store.url'\n" +
		"extore_key_file='/tmp/extore-key'\nextore_redirect_uri='https://shop.example/store/manage'\n"
	const plugin = "[store.extore_plugin]\nhost_library='/usr/lib/lmm/extensions/extore/libextore_plugin_host.so'\n" +
		"plugin_library='/usr/lib/lmm/extensions/extore/libextore_commerce_plugin.so'\n"
	write := func(raw string) string {
		t.Helper()
		path := filepath.Join(t.TempDir(), "extensions.toml")
		if err := os.WriteFile(path, []byte(raw), 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	cfg, err := LoadConfig(write(base))
	if err != nil || cfg.Store.ExtorePlugin.Enabled() {
		t.Fatal("an unconfigured plugin must retain the existing transport", err)
	}
	cfg, err = LoadConfig(write(base + plugin))
	if err != nil || !cfg.Store.ExtorePlugin.Enabled() {
		t.Fatal("explicit native plugin configuration was not retained", err)
	}
	for _, raw := range []string{
		base + "[store.extore_plugin]\nhost_library='/tmp/host.so'\n",
		base + "[store.extore_plugin]\nca_file='/tmp/ca.pem'\n",
		base + strings.Replace(plugin, "'/usr/lib/lmm/extensions/extore/libextore_plugin_host.so'", "'relative.so'", 1),
		base + strings.Replace(plugin, "host_library=", "Host_Library=", 1),
		strings.Replace(base, "extore_key_file='/tmp/extore-key'\n", "", 1) + plugin,
		base + plugin + "fallback=true\n",
	} {
		if _, err := LoadConfig(write(raw)); err == nil {
			t.Fatal("accepted invalid explicit native plugin configuration")
		}
	}
}
