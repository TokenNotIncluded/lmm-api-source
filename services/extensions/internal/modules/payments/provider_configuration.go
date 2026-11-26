// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package payments

import (
	"sort"
	"strconv"
)

// ProviderGatewayConfiguration is the owner-facing view of loaded outbound
// adapters. Credentials, secret paths and raw provider documents stay private.
type ProviderGatewayConfiguration struct {
	Contract         string                   `json:"contract"`
	CallbacksEnabled bool                     `json:"callbacks_enabled"`
	Bindings         []ProviderGatewayBinding `json:"bindings"`
}

type ProviderGatewayBinding struct {
	ChannelID            string `json:"channel_id"`
	Provider             string `json:"provider"`
	MerchantID           string `json:"merchant_id"`
	Environment          string `json:"environment"`
	Currency             string `json:"currency"`
	WalletEnabled        bool   `json:"wallet_enabled"`
	SubscriptionEnabled  *bool  `json:"subscription_enabled,omitempty"`
	APIVersion           string `json:"api_version,omitempty"`
	APIKeyConfigured     bool   `json:"api_key_configured"`
	PrivateKeyConfigured *bool  `json:"private_key_configured,omitempty"`
	ProductID            string `json:"product_id,omitempty"`
	ProductAmountMinor   string `json:"product_amount_minor,omitempty"`
	PayMethodType        string `json:"pay_method_type,omitempty"`
	PayMethodName        string `json:"pay_method_name,omitempty"`
	SuccessURL           string `json:"success_url,omitempty"`
	CancelURL            string `json:"cancel_url,omitempty"`
	NotifyURL            string `json:"notify_url,omitempty"`
	ReturnURL            string `json:"return_url,omitempty"`
}

// DescribeProviderGateways receives only the successfully loaded adapters.
// EPay and Pancake use their own management contracts.
func DescribeProviderGateways(adapters []Adapter, subscriptionAdmission map[int64]bool, callbacksEnabled bool) *ProviderGatewayConfiguration {
	result := &ProviderGatewayConfiguration{
		Contract: "extensions.provider-gateways.v1", CallbacksEnabled: callbacksEnabled,
		Bindings: make([]ProviderGatewayBinding, 0),
	}
	for _, adapter := range adapters {
		channel := adapter.Channel()
		binding := ProviderGatewayBinding{
			ChannelID: strconv.FormatInt(channel.ID, 10), Provider: channel.Provider,
			MerchantID: channel.Merchant, Environment: channel.Environment, Currency: channel.Currency,
			WalletEnabled: channel.Enabled,
		}
		subscriptionEnabled := subscriptionAdmission[channel.ID]
		switch provider := adapter.(type) {
		case *Stripe:
			config := provider.cfg
			binding.APIKeyConfigured = config.APIKey != ""
			binding.APIVersion = config.APIVersion
			binding.SuccessURL, binding.CancelURL = config.SuccessURL, config.CancelURL
			binding.SubscriptionEnabled = &subscriptionEnabled
		case *Creem:
			config := provider.cfg
			binding.APIKeyConfigured = config.APIKey != ""
			binding.ProductID, binding.SuccessURL = config.ProductID, config.SuccessURL
			if config.ProductAmountMinor != 0 {
				binding.ProductAmountMinor = strconv.FormatInt(config.ProductAmountMinor, 10)
			}
			binding.SubscriptionEnabled = &subscriptionEnabled
		case *Waffo:
			config := provider.cfg
			binding.APIKeyConfigured = config.APIKey != ""
			privateKeyConfigured := config.PrivateKey != ""
			binding.PrivateKeyConfigured = &privateKeyConfigured
			binding.NotifyURL, binding.ReturnURL = config.NotifyURL, config.ReturnURL
			binding.PayMethodType, binding.PayMethodName = config.PayMethodType, config.PayMethodName
		case *Coinbase:
			config := provider.cfg
			binding.APIKeyConfigured = config.APIKey != ""
			privateKeyConfigured := provider.key != nil
			binding.PrivateKeyConfigured = &privateKeyConfigured
			binding.SuccessURL, binding.CancelURL = config.SuccessURL, config.CancelURL
		default:
			continue
		}
		result.Bindings = append(result.Bindings, binding)
	}
	sort.Slice(result.Bindings, func(i, j int) bool {
		left, right := result.Bindings[i].ChannelID, result.Bindings[j].ChannelID
		if len(left) != len(right) {
			return len(left) < len(right)
		}
		return left < right
	})
	return result
}
