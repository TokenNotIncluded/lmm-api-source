// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package composition

import (
	"context"
	"errors"
	"slices"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
)

func validateMarketReviewSelection(cfg Config) error {
	if cfg.MarketReviewSocket == "" && cfg.MarketReviewTokenFile == "" && cfg.MarketReviewSettingsFile == "" {
		return nil
	}
	if cfg.MarketReviewSocket == "" || cfg.MarketReviewTokenFile == "" || cfg.MarketReviewSettingsFile == "" {
		return errors.New("market review requires Core socket, separate token and settings file")
	}
	if !slices.Contains(cfg.Selected, "toolmarket") && !slices.Contains(cfg.Selected, "store") {
		return errors.New("market review requires a selected market module")
	}
	return nil
}

func (r *Runtime) composeMarketReview(ctx context.Context, cfg Config, credentials ...[]byte) error {
	if cfg.MarketReviewTokenFile == "" {
		return nil
	}
	token, err := coreclient.ReadTokenFile(cfg.MarketReviewTokenFile)
	if err != nil {
		return err
	}
	credentials = append(credentials, cfg.HostCredential)
	for _, path := range []string{cfg.ToolMarketTokenFile, cfg.ToolMarketWitnessTokenFile, cfg.UserReadTokenFile} {
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
	core, err := marketreview.NewCore(cfg.MarketReviewSocket, token)
	if err != nil {
		return err
	}
	r.close = append(r.close, func() error { core.Close(); return nil })
	if _, err = core.Route(ctx); err != nil {
		return errors.New("Core market review capability unavailable")
	}
	policy, err := marketreview.NewFilePolicy(cfg.MarketReviewSettingsFile)
	if err != nil {
		return err
	}
	if err = policy.With(ctx, func(marketreview.Modes) error { return nil }); err != nil {
		return err
	}
	r.MarketReview = &marketreview.Engine{Policy: policy, Classifier: core}
	if r.Store != nil {
		if err = r.Store.ConfigureAIReview(r.MarketReview); err != nil {
			return err
		}
	}
	if r.ToolMarket != nil {
		if err = r.ToolMarket.ConfigureAIReview(r.MarketReview); err != nil {
			return err
		}
	}
	return nil
}
