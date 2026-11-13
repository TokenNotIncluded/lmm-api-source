package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestStorePickupMailClosedConfiguration(t *testing.T) {
	base := `version=1
[host]
token_file='/private/host'
modules=['store']
[store]
database_name='lmm_store_test'
database_url_file='/private/db'
[store.pickup_mail]
smtp_host='smtp.example.test'
smtp_port=465
tls_mode='implicit'
from='store@example.test'
origin='https://store.example.test'
key_file='/private/mail'
`
	path := filepath.Join(t.TempDir(), "extensions.toml")
	for _, tc := range []struct {
		name, raw string
		valid     bool
	}{
		{"valid", base, true}, {"starttls", strings.Replace(base, "implicit", "starttls", 1), true},
		{"partial", strings.Replace(base, "key_file='/private/mail'", "", 1), false},
		{"plaintext", strings.Replace(base, "implicit", "plain", 1), false},
		{"origin", strings.Replace(base, "https://store.example.test", "http://store.example.test", 1), false},
		{"shared", strings.Replace(base, "/private/mail", "/private/host", 1), false},
		{"auth-half", base + "username='synthetic'\n", false},
		{"insecure", base + "insecure_skip_verify=true\n", false},
		{"disabled-store", strings.Replace(base, "modules=['store']", "modules=['support']", 1), false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if os.WriteFile(path, []byte(tc.raw), 0600) != nil {
				t.Fatal("write fixture")
			}
			_, err := LoadConfig(path)
			if (err == nil) != tc.valid {
				t.Fatalf("valid=%v err=%v", tc.valid, err)
			}
		})
	}
}
