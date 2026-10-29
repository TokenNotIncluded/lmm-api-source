package app

import "errors"

func validateAssistantConfiguration(cfg GoConfig) error {
	key := cfg.Core.AssistantTokenFile
	selected := false
	for _, name := range cfg.Host.Modules {
		selected = selected || name == "assistant"
	}
	if cfg.Assistant.AgentRequired && (!selected || key == "" || cfg.Core.Socket == "" || cfg.Core.IdentityTokenFile == "" || cfg.Assistant.DatabaseName == "" || cfg.Assistant.DatabaseURLFile == "" || cfg.Assistant.CursorKeyFile == "") {
		return errors.New("required assistant agent needs the selected module, core credentials, database and cursor key")
	}
	if key == "" {
		return nil
	}
	if !selected {
		return errors.New("assistant RPC credential requires explicit assistant selection")
	}
	if !absolutePath(key) {
		return errors.New("assistant RPC credential path must be absolute")
	}
	for _, other := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, cfg.Assistant.CursorKeyFile} {
		if other == key {
			return errors.New("assistant RPC credential must be independent")
		}
	}
	return nil
}
