// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package composition

import (
	"context"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/herosms"
	"slices"
)

func validateHeroSMSSelection(cfg Config) error {
	selected := slices.Contains(cfg.Selected, "hero-sms")
	if !selected && cfg.HeroSMSSocket == "" && cfg.HeroSMSTokenFile == "" {
		return nil
	}
	if !selected || cfg.HeroSMSSocket == "" || cfg.HeroSMSTokenFile == "" {
		return errors.New("hero-sms requires explicit selection, Core socket and separate credential")
	}
	return nil
}
func (r *Runtime) composeHeroSMS(ctx context.Context, cfg Config, credentials ...[]byte) error {
	if !slices.Contains(cfg.Selected, "hero-sms") {
		return nil
	}
	token, err := coreclient.ReadTokenFile(cfg.HeroSMSTokenFile)
	if err != nil {
		return err
	}
	credentials = append(credentials, cfg.HostCredential)
	for _, path := range []string{cfg.ToolMarketTokenFile, cfg.ToolMarketWitnessTokenFile, cfg.UserReadTokenFile, cfg.MarketReviewTokenFile, cfg.DirectoryAdTokenFile} {
		if path != "" {
			other, e := coreclient.ReadTokenFile(path)
			if e != nil {
				return e
			}
			credentials = append(credentials, other)
		}
	}
	if err = independentRuntimeCredentials(append([][]byte{token}, credentials...)...); err != nil {
		return err
	}
	core, err := herosms.NewCore(cfg.HeroSMSSocket, token)
	if err != nil {
		return err
	}
	r.close = append(r.close, func() error { core.Close(); return nil })
	if err = core.Ready(ctx); err != nil {
		return errors.New("Core HeroSMS capability unavailable")
	}
	r.HeroSMS = &herosms.Service{Core: core}
	return nil
}
