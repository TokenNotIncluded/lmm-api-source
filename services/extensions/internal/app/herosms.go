// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package app

import (
	"errors"
	"slices"
)

type HeroSMSConfig struct {
	CoreSocket    string `toml:"core_socket"`
	CoreTokenFile string `toml:"core_token_file"`
}

func validateHeroSMSConfiguration(cfg GoConfig) error {
	h := cfg.HeroSMS
	selected := slices.Contains(cfg.Host.Modules, "hero-sms")
	if !selected && h == (HeroSMSConfig{}) {
		return nil
	}
	if !selected || !absolutePath(h.CoreSocket) || !absolutePath(h.CoreTokenFile) {
		return errors.New("hero_sms requires selected hero-sms and absolute core_socket/core_token_file")
	}
	for _, path := range []string{cfg.Host.TokenFile, cfg.Host.PluginStateFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.AssistantTokenFile, cfg.Core.PaymentTokenFile, cfg.Core.UserReadTokenFile, cfg.ToolMarket.CoreTokenFile, cfg.ToolMarket.EncryptionKeyFile, cfg.ToolMarket.FixedWitnessTokenFile, cfg.Store.StockKeyFile, cfg.Store.ExtoreKeyFile, cfg.MarketReview.CoreTokenFile, cfg.MarketReview.SettingsFile} {
		if path != "" && path == h.CoreTokenFile {
			return errors.New("HeroSMS credential path must be independent")
		}
	}
	return nil
}
