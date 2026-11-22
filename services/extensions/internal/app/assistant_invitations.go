package app

import (
	"errors"
	"net/url"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
)

func validateAssistantInvitationConfiguration(cfg GoConfig) error {
	c := cfg.Assistant.InvitationMail
	if !c.Enabled() {
		return nil
	}
	if !cfg.Assistant.AgentRequired || c.Validate() != nil {
		return errors.New("assistant invitation mail requires the installed agent and complete TLS SMTP settings")
	}
	if (c.PasswordFile != "" && !absolutePath(c.PasswordFile)) || (c.CAFile != "" && !absolutePath(c.CAFile)) {
		return errors.New("assistant invitation mail requires absolute private file paths")
	}
	u, e := url.Parse(cfg.Host.PublicOrigin)
	if e != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return errors.New("assistant invitation mail requires the configured HTTPS public origin")
	}
	return nil
}

func assistantInvitationSender(cfg GoConfig) (assistant_services.InvitationSender, error) {
	if !cfg.Assistant.InvitationMail.Enabled() {
		return nil, nil
	}
	if e := validateAssistantInvitationConfiguration(cfg); e != nil {
		return nil, e
	}
	c := cfg.Assistant.InvitationMail
	var password, ca []byte
	var e error
	if c.PasswordFile != "" {
		password, e = coreclient.ReadPrivateFile(c.PasswordFile, 4096)
		if e != nil {
			return nil, errors.New("assistant invitation SMTP credential unavailable")
		}
	}
	if c.CAFile != "" {
		ca, e = coreclient.ReadPrivateFile(c.CAFile, 1<<20)
		if e != nil {
			return nil, errors.New("assistant invitation SMTP CA unavailable")
		}
	}
	sender, e := assistant_services.NewInvitationSMTP(c, string(password), ca, cfg.Host.PublicOrigin)
	if e != nil {
		return nil, errors.New("assistant invitation SMTP settings invalid")
	}
	return sender, nil
}
