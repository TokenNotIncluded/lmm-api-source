package app

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

func searchConfig(t *testing.T, search string) GoConfig {
	t.Helper()
	cfg, err := loadSearchConfig(t, search)
	if err != nil {
		t.Fatal(err)
	}
	return cfg
}

func loadSearchConfig(t *testing.T, search string) (GoConfig, error) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "extensions.toml")
	text := "version=1\n[host]\ntoken_file='/tmp/host'\nmodules=['assistant']\n" +
		"[core]\nsocket='/tmp/core.sock'\nidentity_token_file='/tmp/identity'\nassistant_token_file='/tmp/assistant'\n" +
		"[assistant]\ndatabase_name='assistant_db'\ndatabase_url_file='/tmp/assistant-db'\ncursor_key_file='/tmp/cursor'\nagent_required=true\n" + search
	if err := os.WriteFile(path, []byte(text), 0600); err != nil {
		t.Fatal(err)
	}
	return LoadConfig(path)
}

func TestAssistantSearchConfigurationIsExplicitAndPrivate(t *testing.T) {
	const settings = "[assistant.search]\nprovider='exa'\napi_key_file='/tmp/exa-key'\ntimeout_ms=1250\n"
	cfg := searchConfig(t, settings)
	if cfg.Assistant.Search.Provider != "exa" || cfg.Assistant.Search.TimeoutMS != 1250 {
		t.Fatal("search configuration was not loaded")
	}
	for _, text := range []string{
		strings.Replace(settings, "provider='exa'", "provider='tavily'", 1),
		strings.Replace(settings, "provider='exa'", "provider='none'", 1),
		strings.Replace(settings, "api_key_file='/tmp/exa-key'", "api_key_file='relative'", 1),
		strings.Replace(settings, "api_key_file='/tmp/exa-key'", "api_key='do-not-put-secrets-in-config'", 1),
		strings.Replace(settings, "api_key_file", "APIKeyFile", 1),
		strings.Replace(settings, "timeout_ms=1250", "timeout_ms=30001", 1),
		strings.Replace(settings, "timeout_ms=1250", "timeout_ms=-1", 1),
	} {
		if _, err := loadSearchConfig(t, text); err == nil {
			t.Fatal("accepted invalid or ignored search configuration")
		}
	}
	cfg.Assistant.AgentRequired = false
	if validateAssistantConfiguration(cfg) == nil {
		t.Fatal("accepted search without the installed agent")
	}
	for _, text := range []string{"", "[assistant.search]\nprovider='none'\n"} {
		dependencies, err := assistantServiceDependencies(searchConfig(t, text))
		if err != nil || dependencies != nil {
			t.Fatal("unconfigured search would advertise an unavailable dependency")
		}
	}
}

func TestAssistantSearchInjectsPrivateExaWithoutChangingTransportPolicy(t *testing.T) {
	const secret = "synthetic-exa-private-key"
	key := filepath.Join(t.TempDir(), "exa-key")
	if err := os.WriteFile(key, []byte(secret+"\n"), 0600); err != nil {
		t.Fatal(err)
	}
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.Method != http.MethodPost || r.Header.Get("X-Api-Key") != secret {
			t.Error("configured Exa transport was not used")
		}
		var input map[string]any
		if json.NewDecoder(r.Body).Decode(&input) != nil || input["query"] != "bounded query" {
			t.Error("incorrect search query")
		}
		fmt.Fprintf(w, `{"results":[{"text":%q}]}`, secret)
	}))
	defer server.Close()
	cfg := searchConfig(t, fmt.Sprintf("[assistant.search]\nprovider='exa'\napi_key_file=%q\nendpoint=%q\ntimeout_ms=1000\n", key, server.URL))
	if _, err := assistantServiceDependencies(cfg); err == nil {
		t.Fatal("ordinary host configuration allowed a loopback search endpoint")
	}
	cfg.Host.LocalHTTP = true
	factory, err := assistantServiceDependencies(cfg)
	if err != nil || factory == nil || calls.Load() != 0 {
		t.Fatal("dependency preparation dispatched or failed")
	}
	dependencies, err := factory(context.Background(), nil)
	if err != nil || dependencies.Search == nil {
		t.Fatal("actual provider was not injected")
	}
	raw, err := dependencies.Search.Search(context.Background(), "bounded query")
	if err != nil || calls.Load() != 1 || strings.Contains(string(raw), secret) {
		t.Fatal("search result failed, retried or exposed a secret")
	}
	var result map[string]any
	if json.Unmarshal(raw, &result) != nil || result["ok"] != true || result["source_is_untrusted"] != true {
		t.Fatal("search result trust boundary changed")
	}
}

func TestAssistantSearchCredentialFailuresDoNotExposeSecrets(t *testing.T) {
	path := filepath.Join(t.TempDir(), "synthetic-private-credential")
	cfg := searchConfig(t, fmt.Sprintf("[assistant.search]\nprovider='exa'\napi_key_file=%q\n", path))
	for _, contents := range []string{"", "synthetic-secret\nembedded", strings.Repeat("s", 4097)} {
		if err := os.WriteFile(path, []byte(contents), 0600); err != nil {
			t.Fatal(err)
		}
		_, err := assistantServiceDependencies(cfg)
		if err == nil || strings.Contains(err.Error(), path) || strings.Contains(err.Error(), "synthetic-secret") {
			t.Fatal("invalid private search credential accepted or disclosed")
		}
	}
	if err := os.WriteFile(path, []byte("synthetic-private-value"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := assistantServiceDependencies(cfg); err == nil || strings.Contains(err.Error(), path) {
		t.Fatal("unsafe credential permissions accepted or path disclosed")
	}
}
