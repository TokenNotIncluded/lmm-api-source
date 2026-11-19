package app

import "errors"

func extorePluginConfigurationKey(key string) bool {
	switch key {
	case "store.extore_plugin", "store.extore_plugin.origin", "store.extore_plugin.socket",
		"store.extore_plugin.host", "store.extore_plugin.library", "store.extore_plugin.sha256",
		"store.extore_plugin.data_dir":
		return true
	}
	return false
}

func validateStoreExtorePlugin(cfg GoConfig) error {
	if err := cfg.Store.ExtorePlugin.Validate(); err != nil {
		return err
	}
	if cfg.Store.ExtorePlugin.Enabled() && (cfg.Store.ExtoreKeyFile == "" ||
		cfg.Store.ExtoreRedirectURI == "" || cfg.Store.DatabaseName == "" || cfg.Store.DatabaseURLFile == "") {
		return errors.New("store Extore plugin requires configured Extore authorization and store storage")
	}
	return nil
}
