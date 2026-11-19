package composition

import (
	"context"
	"errors"
	"slices"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

func validateExtorePluginSelection(cfg Config) error {
	if err := cfg.StoreExtorePlugin.Validate(); err != nil {
		return err
	}
	if cfg.StoreExtorePlugin.Enabled() && (!slices.Contains(cfg.Selected, "store") ||
		cfg.StoreExtoreKeyFile == "" || cfg.StoreExtoreRedirectURI == "" ||
		cfg.Store.URLFile == "" || cfg.Store.Database == "") {
		return errors.New("Extore plugin requires the selected store and configured Extore authorization")
	}
	return nil
}

func (r *Runtime) composeExtorePlugin(ctx context.Context, cfg Config) error {
	if !cfg.StoreExtorePlugin.Enabled() {
		return nil
	}
	host, err := extore.StartPlugin(ctx, cfg.StoreExtorePlugin)
	if err != nil {
		return err
	}
	r.close = append(r.close, host.Close)
	return r.Store.WithExtorePlugin(host)
}
