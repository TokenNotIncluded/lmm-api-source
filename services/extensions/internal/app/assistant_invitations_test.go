package app

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func invitationConfig(t *testing.T, mail string) (GoConfig, error) {
	t.Helper()
	p := filepath.Join(t.TempDir(), "extensions.toml")
	text := "version=1\n[host]\ntoken_file='/tmp/host'\nmodules=['assistant']\npublic_origin='https://console.example.invalid'\n" +
		"[core]\nsocket='/tmp/core.sock'\nidentity_token_file='/tmp/identity'\nassistant_token_file='/tmp/assistant'\n" +
		"[assistant]\ndatabase_name='assistant_db'\ndatabase_url_file='/tmp/assistant-db'\ncursor_key_file='/tmp/cursor'\nagent_required=true\n" + mail
	if e := os.WriteFile(p, []byte(text), 0600); e != nil {
		t.Fatal(e)
	}
	return LoadConfig(p)
}
func TestAssistantInvitationConfigurationIsExplicitPrivateAndOptional(t *testing.T) {
	settings := "[assistant.invitation_mail]\nsmtp_host='smtp.example.invalid'\nsmtp_port=465\ntls_mode='implicit'\nfrom='sender@example.invalid'\nusername='sender'\npassword_file='/tmp/private-invitation-password'\n"
	cfg, e := invitationConfig(t, settings)
	if e != nil {
		t.Fatal(e)
	}
	if !cfg.Assistant.InvitationMail.Enabled() || cfg.Assistant.InvitationMail.TLSMode != "implicit" {
		t.Fatal("mail settings not loaded")
	}
	for _, bad := range []string{
		strings.Replace(settings, "password_file=", "password=", 1),
		strings.Replace(settings, "password_file='/tmp/private-invitation-password'", "password_file='relative'", 1),
		strings.Replace(settings, "tls_mode='implicit'", "tls_mode='plain'", 1),
		strings.Replace(settings, "smtp_port=465", "smtp_port=0", 1),
		strings.Replace(settings, "from='sender@example.invalid'", "from='Display <sender@example.invalid>'", 1),
		strings.Replace(settings, "smtp_host=", "SMTPHost=", 1),
	} {
		if _, e := invitationConfig(t, bad); e == nil {
			t.Fatal("unsafe or ignored SMTP config accepted")
		}
	}
	cfg.Assistant.AgentRequired = false
	if validateAssistantInvitationConfiguration(cfg) == nil {
		t.Fatal("mail enabled without agent")
	}
	cfg, e = invitationConfig(t, "")
	if e != nil {
		t.Fatal(e)
	}
	factory, e := assistantServiceDependencies(cfg)
	if e != nil || factory != nil {
		t.Fatal("unconfigured mail advertised")
	}
}
func TestAssistantInvitationInjectsOnlyPrivateSenderAndPreservesSearch(t *testing.T) {
	dir := t.TempDir()
	password := filepath.Join(dir, "smtp.secret")
	search := filepath.Join(dir, "exa.secret")
	if e := os.WriteFile(password, []byte("synthetic-invitation-private-password"), 0600); e != nil {
		t.Fatal(e)
	}
	if e := os.WriteFile(search, []byte("synthetic-exa-private-secret"), 0600); e != nil {
		t.Fatal(e)
	}
	settings := fmt.Sprintf("[assistant.invitation_mail]\nsmtp_host='smtp.example.invalid'\nsmtp_port=465\ntls_mode='implicit'\nfrom='sender@example.invalid'\nusername='sender'\npassword_file=%q\n", password)
	cfg, e := invitationConfig(t, settings)
	if e != nil {
		t.Fatal(e)
	}
	factory, e := assistantServiceDependencies(cfg)
	if e != nil || factory == nil {
		t.Fatal("configured mail not injected", e)
	}
	deps, e := factory(context.Background(), nil)
	if e != nil || deps.InvitationMail == nil || deps.Search != nil || deps.Confirmations != nil || deps.Invitations != nil {
		t.Fatal("SMTP config forged Core readiness")
	}
	cfg, e = invitationConfig(t, settings+fmt.Sprintf("[assistant.search]\nprovider='exa'\napi_key_file=%q\n", search))
	if e != nil {
		t.Fatal(e)
	}
	factory, e = assistantServiceDependencies(cfg)
	if e != nil {
		t.Fatal(e)
	}
	deps, e = factory(context.Background(), nil)
	if e != nil || deps.InvitationMail == nil || deps.Search == nil {
		t.Fatal("independent configured port overwritten")
	}
}
func TestAssistantInvitationCredentialFailuresOmitPrivateData(t *testing.T) {
	p := filepath.Join(t.TempDir(), "private-smtp-path")
	cfg, e := invitationConfig(t, fmt.Sprintf("[assistant.invitation_mail]\nsmtp_host='smtp.example.invalid'\nsmtp_port=465\ntls_mode='implicit'\nfrom='sender@example.invalid'\nusername='sender'\npassword_file=%q\n", p))
	if e != nil {
		t.Fatal(e)
	}
	for _, mode := range []os.FileMode{0644, 0600} {
		body := "synthetic-private-smtp-secret"
		if mode == 0600 {
			body = ""
		}
		if e := os.WriteFile(p, []byte(body), 0600); e != nil {
			t.Fatal(e)
		}
		if e := os.Chmod(p, mode); e != nil {
			t.Fatal(e)
		}
		_, e = assistantServiceDependencies(cfg)
		if e == nil || strings.Contains(e.Error(), p) || strings.Contains(e.Error(), "synthetic-private-smtp-secret") {
			t.Fatal("unsafe SMTP secret accepted or exposed")
		}
	}
	cfg.Host.PublicOrigin = "https://console.example.invalid/arbitrary"
	if _, e := assistantServiceDependencies(cfg); e == nil {
		t.Fatal("invitation origin path accepted")
	}
}
