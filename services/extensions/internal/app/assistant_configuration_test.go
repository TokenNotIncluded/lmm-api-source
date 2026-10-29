package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAssistantConfigurationRequiresExplicitIndependentCredential(t *testing.T) {
	cfg := GoConfig{Host: HostConfig{Modules: []string{"assistant"}, TokenFile: "/tmp/host-token"}, Core: CoreConfig{AssistantTokenFile: "/tmp/assistant-token", IdentityTokenFile: "/tmp/identity-token", CommerceTokenFile: "/tmp/commerce-token", PaymentTokenFile: "/tmp/payment-token"}}
	if err := validateAssistantConfiguration(cfg); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, "relative"} {
		mutated := cfg
		mutated.Core.AssistantTokenFile = path
		if validateAssistantConfiguration(mutated) == nil {
			t.Fatal("aliased or relative assistant credential accepted")
		}
	}
	cfg.Host.Modules = nil
	if validateAssistantConfiguration(cfg) == nil {
		t.Fatal("ignored assistant credential without module selection")
	}
	cfg.Core.AssistantTokenFile = ""
	if validateAssistantConfiguration(cfg) != nil {
		t.Fatal("missing optional assistant credential stopped other modules")
	}
}

func TestRequiredAgentConfigRequiresItsDependencies(t *testing.T) {
	base := "version=1\n[host]\ntoken_file='/tmp/host'\nmodules=['assistant']\n" +
		"[core]\nsocket='/tmp/core.sock'\nidentity_token_file='/tmp/identity'\nassistant_token_file='/tmp/assistant'\n" +
		"[assistant]\ndatabase_name='assistant_db'\ndatabase_url_file='/tmp/assistant-db'\ncursor_key_file='/tmp/cursor'\nagent_required=true\n"
	load := func(text string) (GoConfig, error) {
		t.Helper()
		path := filepath.Join(t.TempDir(), "extensions.toml")
		if err := os.WriteFile(path, []byte(text), 0600); err != nil {
			t.Fatal(err)
		}
		return LoadConfig(path)
	}
	cfg, err := load(base)
	if err != nil || !cfg.Assistant.AgentRequired {
		t.Fatalf("complete agent configuration rejected: %v", err)
	}
	for _, line := range []string{
		"assistant_token_file='/tmp/assistant'\n",
		"database_name='assistant_db'\n",
		"database_url_file='/tmp/assistant-db'\n",
		"cursor_key_file='/tmp/cursor'\n",
	} {
		if _, err := load(strings.Replace(base, line, "", 1)); err == nil {
			t.Fatalf("accepted required agent without %s", line)
		}
	}
	if _, err := load(strings.Replace(base, "modules=['assistant']", "modules=['none']", 1)); err == nil {
		t.Fatal("accepted required agent without selected assistant")
	}
	if _, err := load(strings.Replace(base, "agent_required=true", "agent_required='true'", 1)); err == nil {
		t.Fatal("accepted non-boolean agent requirement")
	}
	cfg, err = load(strings.Replace(base, "agent_required=true\n", "", 1))
	if err != nil || cfg.Assistant.AgentRequired {
		t.Fatalf("optional agent default changed: %v", err)
	}
}
