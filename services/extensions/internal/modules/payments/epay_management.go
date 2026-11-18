// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package payments

import (
	"context"
	"encoding/hex"
	"strconv"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

type EPayConfigurationSource interface {
	PaymentEPayConfiguration(context.Context) (*pb.EPayConfiguration, error)
}

// EPayAdministration reads the effective Core policy. Signing keys and their
// paths never cross this port; Go retains only its local admission switches.
type EPayAdministration struct {
	source                EPayConfigurationSource
	wallet                *Service
	subscriptionAdmission map[int64]bool
}

func NewEPayAdministration(source EPayConfigurationSource, wallet *Service, subscriptions map[int64]bool) (*EPayAdministration, error) {
	if source == nil || wallet == nil {
		return nil, ErrInvalid
	}
	admission := make(map[int64]bool, len(subscriptions))
	for id, enabled := range subscriptions {
		admission[id] = enabled
	}
	return &EPayAdministration{source: source, wallet: wallet, subscriptionAdmission: admission}, nil
}

type epayConfiguration struct {
	Contract string                     `json:"contract"`
	Revision string                     `json:"revision"`
	Bindings []epayConfigurationBinding `json:"bindings"`
}

type epayConfigurationBinding struct {
	ChannelID           int64  `json:"channel_id"`
	MerchantID          string `json:"merchant_id"`
	Environment         string `json:"environment"`
	Currency            string `json:"currency"`
	PaymentType         string `json:"payment_type"`
	Endpoint            string `json:"endpoint"`
	NotifyURL           string `json:"notify_url"`
	ReturnURL           string `json:"return_url"`
	KeyConfigured       bool   `json:"key_configured"`
	Enabled             bool   `json:"enabled"`
	Editable            bool   `json:"editable"`
	CreditsPerMinor     string `json:"credits_per_minor"`
	MaxAmountMinor      string `json:"max_amount_minor"`
	MinAmountMinor      string `json:"min_amount_minor"`
	AdapterConfigured   bool   `json:"adapter_configured"`
	AdapterMatches      bool   `json:"adapter_matches"`
	WalletEnabled       bool   `json:"wallet_enabled"`
	SubscriptionEnabled bool   `json:"subscription_enabled"`
}

func (a *EPayAdministration) Configuration(ctx context.Context) (any, error) {
	policy, err := a.source.PaymentEPayConfiguration(ctx)
	if err != nil || policy == nil || len(policy.Revision) != 64 || len(policy.Bindings) > 16 {
		return nil, ErrUnavailable
	}
	if _, err := hex.DecodeString(policy.Revision); err != nil {
		return nil, ErrUnavailable
	}
	result := epayConfiguration{
		Contract: "core.epay-configuration.v1", Revision: policy.Revision,
		Bindings: make([]epayConfigurationBinding, 0, len(policy.Bindings)),
	}
	adapters := a.wallet.adapterSnapshot()
	seen := map[int64]bool{}
	for _, binding := range policy.Bindings {
		if binding == nil || binding.ChannelId <= 0 || seen[binding.ChannelId] {
			return nil, ErrUnavailable
		}
		seen[binding.ChannelId] = true
		minimum := binding.MinAmountMinor
		if minimum == "" {
			minimum = "1" // Earlier protocol projections had the default one-minor floor.
		}
		floor, floorErr := strconv.ParseInt(minimum, 10, 64)
		maximum, maxErr := strconv.ParseInt(binding.MaxAmountMinor, 10, 64)
		if floorErr != nil || maxErr != nil || floor < 1 || floor > maximum || floor > maxAmount || strconv.FormatInt(floor, 10) != minimum {
			return nil, ErrUnavailable
		}
		current, configured := adapters[binding.ChannelId].(*EPay)
		localEnabled, matches := false, false
		if configured {
			channel := current.Channel()
			localEnabled = channel.Enabled
			matches = channel.Merchant == binding.MerchantId &&
				channel.Environment == binding.Environment && channel.Currency == binding.Currency &&
				current.cfg.PaymentType == binding.PaymentType
		}
		subscriptions := a.subscriptionAdmission[binding.ChannelId]
		result.Bindings = append(result.Bindings, epayConfigurationBinding{
			ChannelID: binding.ChannelId, MerchantID: binding.MerchantId,
			Environment: binding.Environment, Currency: binding.Currency, PaymentType: binding.PaymentType,
			Endpoint: binding.Endpoint, NotifyURL: binding.NotifyUrl, ReturnURL: binding.ReturnUrl,
			KeyConfigured: binding.KeyConfigured, Enabled: binding.Enabled,
			Editable: binding.Editable && !binding.Enabled && binding.Currency == "LDC" &&
				binding.PaymentType == "epay" && configured && matches && !localEnabled && !subscriptions,
			CreditsPerMinor: binding.CreditsPerMinor, MaxAmountMinor: binding.MaxAmountMinor, MinAmountMinor: minimum,
			AdapterConfigured: configured, AdapterMatches: matches,
			WalletEnabled: localEnabled, SubscriptionEnabled: subscriptions,
		})
	}
	return result, nil
}
