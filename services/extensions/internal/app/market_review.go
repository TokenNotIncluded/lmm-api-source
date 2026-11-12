// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package app

import (
	"errors"
	"slices"
)

type MarketReviewConfig struct {
	CoreSocket    string `toml:"core_socket"`
	CoreTokenFile string `toml:"core_token_file"`
	SettingsFile  string `toml:"settings_file"`
}

func validateMarketReviewConfiguration(cfg GoConfig) error {
	m := cfg.MarketReview
	if m == (MarketReviewConfig{}) {
		return nil
	}
	if !absolutePath(m.CoreSocket) || !absolutePath(m.CoreTokenFile) || !absolutePath(m.SettingsFile) {
		return errors.New("market_review requires absolute core_socket, core_token_file and settings_file")
	}
	if !cfg.Host.WebCompat || !slices.Contains(cfg.Host.Modules, "store") && !slices.Contains(cfg.Host.Modules, "toolmarket") {
		return errors.New("market_review requires web_compat and a selected market module")
	}
	if m.CoreTokenFile == m.SettingsFile {
		return errors.New("market review settings and credential must be independent")
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Host.PluginStateFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.AssistantTokenFile, cfg.Core.PaymentTokenFile, cfg.Core.UserReadTokenFile, cfg.ToolMarket.CoreTokenFile, cfg.ToolMarket.EncryptionKeyFile, cfg.ToolMarket.FixedWitnessTokenFile, cfg.Store.StockKeyFile, cfg.Store.ExtoreKeyFile} {
		if path != "" && (path == m.CoreTokenFile || path == m.SettingsFile) {
			return errors.New("market review paths must be independent")
		}
	}
	return nil
}
