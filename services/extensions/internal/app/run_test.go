package app

import (
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

func TestRunRejectsCoreConfigurationBeforeCredentialIO(t *testing.T) {
	args := os.Args
	os.Args = []string{"lmm-extensions"}
	t.Cleanup(func() { os.Args = args })
	for _, name := range []string{"SQL_DSN", "LOG_SQL_DSN", "DATABASE_URL", "LMM_CORE_DATABASE_URL", "LMM_CORE_DATABASE_URL_FILE", "LMM_DB_MIGRATION_MODE"} {
		t.Setenv(name, "")
	}
	const secret = "private-core-connection-do-not-log"
	t.Setenv("LMM_CORE_DATABASE_URL", secret)
	t.Setenv("LMM_EXTENSION_TOKEN_FILE", "/missing/extension-credential")
	err := Run()
	if err == nil || !strings.Contains(err.Error(), "LMM_CORE_DATABASE_URL") {
		t.Fatalf("must reject core configuration before credential I/O, got %v", err)
	}
	if strings.Contains(err.Error(), secret) || strings.Contains(err.Error(), "open extension") {
		t.Fatalf("unexpected startup error: %v", err)
	}
}

func TestRunRejectsRetiredCommandsBeforeCredentialIO(t *testing.T) {
	args := os.Args
	t.Cleanup(func() { os.Args = args })
	for _, command := range []string{"migrate", "serve", "deploy"} {
		t.Run(command, func(t *testing.T) {
			os.Args = []string{"lmm-extensions", command}
			err := Run()
			if err == nil || !strings.Contains(err.Error(), "accepts no subcommands") {
				t.Fatalf("must reject retired command, got %v", err)
			}
		})
	}
}

// Exercise actual TOML loading, private Core readiness and app assembly. An
// occupied listener stops the process before workers start; reaching that bind
// proves settings do not depend on an unrelated commerce or operations module.
func TestRunAssemblesIndependentHeroSMSWeb(t *testing.T) {
	for _, name := range []string{"SQL_DSN", "LOG_SQL_DSN", "DATABASE_URL", "LMM_CORE_DATABASE_URL", "LMM_CORE_DATABASE_URL_FILE", "LMM_DB_MIGRATION_MODE", "LMM_EXTENSION_TOKEN_FILE", "LMM_EXTENSION_LISTEN", "LMM_EXTENSION_MODULES", "LMM_EXTENSION_WEB_COMPAT", "LMM_EXTENSION_PLUGIN_STATE_FILE", "LMM_CORE_RPC_SOCKET", "LMM_CORE_RPC_TOKEN_FILE", "LMM_CORE_COMMERCE_RPC_TOKEN_FILE", "LMM_STORE_STOCK_KEY_FILE", "LMM_STORE_DATABASE_URL_FILE", "LMM_STORE_DATABASE_NAME", "LMM_SUPPORT_DATABASE_URL_FILE", "LMM_SUPPORT_DATABASE_NAME", "LMM_LEGAL_CONFIG_FILE"} {
		t.Setenv(name, "")
	}
	directory, err := os.MkdirTemp("", "lmm-hero-app-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(directory) })
	private := func(name, data string) string {
		t.Helper()
		path := filepath.Join(directory, name)
		if err := os.WriteFile(path, []byte(data), 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	host := private("host", "host-token-for-web-assembly-11111111")
	identity := private("identity", "identity-token-for-web-assembly-22222222")
	const heroCredential = "hero-token-for-web-assembly-33333333"
	hero := private("hero", heroCredential)
	socket := filepath.Join(directory, "hero.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	var readyCalls atomic.Int32
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/internal/hero-sms/v1/ready" ||
			r.Header.Get("Authorization") != "Bearer "+heroCredential || r.Header.Get("X-LMM-Protocol") != "1" {
			http.Error(w, "invalid readiness request", http.StatusBadRequest)
			return
		}
		readyCalls.Add(1)
		w.Header().Set("Content-Type", "application/json")
		_, _ = fmt.Fprint(w, "{\"protocol\":1,\"service\":\"hero-sms\",\"ready\":true,\"purchases_available\":false}")
	})}
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(func() { _ = server.Close() })
	occupied, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = occupied.Close() })
	originalArgs := os.Args
	t.Cleanup(func() { os.Args = originalArgs })
	for _, withHero := range []bool{false, true} {
		name, selected, section := "identity-only", "[\"identity\"]", ""
		if withHero {
			name, selected = "hero-settings", "[\"identity\",\"hero-sms\"]"
			section = fmt.Sprintf("\n[hero_sms]\ncore_socket=%q\ncore_token_file=%q\n", socket, hero)
		}
		t.Run(name, func(t *testing.T) {
			config := fmt.Sprintf("version=1\n[host]\nlisten=%q\nlocal_http=true\npublic_origin=%q\ntoken_file=%q\nmodules=%s\nweb_compat=true\n[core]\nsocket=%q\nidentity_token_file=%q\n%s",
				occupied.Addr().String(), "http://"+occupied.Addr().String(), host, selected, filepath.Join(directory, "identity.sock"), identity, section)
			path := private(name+".toml", config)
			os.Args = []string{"lmm-extensions", "--config", path}
			before := readyCalls.Load()
			err := Run()
			want, calls := "Web compatibility requires an assembled Web module", int32(0)
			if withHero {
				want, calls = "bind extension listener:", 1
			}
			if err == nil || !strings.Contains(err.Error(), want) || readyCalls.Load()-before != calls {
				t.Fatalf("wrong independent Web startup boundary: %v; readiness calls=%d", err, readyCalls.Load()-before)
			}
		})
	}
}
