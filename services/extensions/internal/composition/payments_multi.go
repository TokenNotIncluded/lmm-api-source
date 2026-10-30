// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"bytes"
	"errors"
	"strings"
	"unicode/utf8"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

// Version 2 names the protocol and environment for each channel. The Core owns
// conversion and verification credentials; this document contains outbound IO
// credentials only. In particular, an EPay merchant key is never loaded here.
type multiPaymentDocument struct {
	Version  int                   `toml:"version"`
	Channels []multiPaymentChannel `toml:"channels"`
}

type multiPaymentChannel struct {
	ID                 int64  `toml:"id"`
	Provider           string `toml:"provider"`
	Merchant           string `toml:"merchant"`
	Environment        string `toml:"environment"`
	Currency           string `toml:"currency"`
	Enabled            bool   `toml:"enabled"`
	APIKeyFile         string `toml:"api_key_file"`
	APIVersion         string `toml:"api_version"`
	PrivateKeyFile     string `toml:"private_key_file"`
	SuccessURL         string `toml:"success_url"`
	CancelURL          string `toml:"cancel_url"`
	Endpoint           string `toml:"endpoint"`
	NotifyURL          string `toml:"notify_url"`
	ReturnURL          string `toml:"return_url"`
	PaymentType        string `toml:"payment_type"`
	ProductID          string `toml:"product_id"`
	ProductAmountMinor int64  `toml:"product_amount_minor"`
	StoreID            string `toml:"store_id"`
	PayMethodType      string `toml:"pay_method_type"`
	PayMethodName      string `toml:"pay_method_name"`
	CheckoutRegion     string `toml:"checkout_region"`
	CheckoutLanguage   string `toml:"checkout_language"`
	TaxCategory        string `toml:"tax_category"`
}

func outboundPaymentSecret(path string, pem bool) (string, error) {
	if !cleanPaymentPath(path) {
		return "", errors.New("invalid private payment credential path")
	}
	limit := 4096
	if pem {
		limit = 16384
	}
	raw, err := coreclient.ReadPrivateFile(path, limit)
	if err != nil || !utf8.Valid(raw) {
		return "", errors.New("private payment credential unavailable")
	}
	secret := strings.TrimSpace(string(raw))
	if len(secret) < 16 {
		return "", errors.New("invalid private payment credential")
	}
	if !pem {
		for _, c := range []byte(secret) {
			if c < 33 || c > 126 {
				return "", errors.New("invalid private payment credential")
			}
		}
	}
	return secret, nil
}

func loadMultiPaymentAdapters(raw []byte, signer payments.EPayCheckoutSigner) ([]payments.Adapter, error) {
	var cfg multiPaymentDocument
	meta, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&cfg)
	if err != nil || len(meta.Undecoded()) != 0 || cfg.Version != 2 || !meta.IsDefined("channels") || len(cfg.Channels) > 16 {
		return nil, errors.New("invalid multi-provider payment configuration")
	}
	// TOML accepts case-insensitive Go field matches; the public contract does not.
	allowed := map[string]bool{"version": true, "channels": true}
	for _, field := range strings.Fields("id provider merchant environment currency enabled api_key_file api_version private_key_file success_url cancel_url endpoint notify_url return_url payment_type product_id product_amount_minor store_id pay_method_type pay_method_name checkout_region checkout_language tax_category") {
		allowed["channels."+field] = true
	}
	for _, key := range meta.Keys() {
		if !allowed[key.String()] {
			return nil, errors.New("unknown multi-provider payment field")
		}
	}
	adapters := make([]payments.Adapter, 0, len(cfg.Channels))
	ids := make(map[int64]bool)
	for _, c := range cfg.Channels {
		if c.ID <= 0 || ids[c.ID] {
			return nil, errors.New("invalid or duplicate payment channel")
		}
		ids[c.ID] = true
		channel := payments.Channel{ID: c.ID, Provider: c.Provider, Merchant: c.Merchant, Environment: c.Environment, Currency: c.Currency, Enabled: c.Enabled}
		var apiKey, privateKey string
		switch c.Provider {
		case "stripe", "creem", "waffo", "coinbase":
			apiKey, err = outboundPaymentSecret(c.APIKeyFile, false)
		case "epay", "waffo_pancake":
			if c.APIKeyFile != "" {
				return nil, errors.New("payment protocol does not accept an API key")
			}
		default:
			return nil, errors.New("unsupported payment provider")
		}
		if err != nil {
			return nil, err
		}
		if c.Provider == "waffo" || c.Provider == "waffo_pancake" || c.Provider == "coinbase" {
			privateKey, err = outboundPaymentSecret(c.PrivateKeyFile, true)
			if err != nil {
				return nil, err
			}
		} else if c.PrivateKeyFile != "" {
			return nil, errors.New("payment protocol does not accept a private signing key")
		}
		var adapter payments.Adapter
		switch c.Provider {
		case "stripe":
			adapter, err = payments.NewStripe(payments.StripeConfig{Channel: channel, APIKey: apiKey, APIVersion: c.APIVersion, SuccessURL: c.SuccessURL, CancelURL: c.CancelURL})
		case "epay":
			adapter, err = payments.NewCoreEPay(payments.EPayConfig{Channel: channel, Endpoint: c.Endpoint, PaymentType: c.PaymentType, NotifyURL: c.NotifyURL, ReturnURL: c.ReturnURL}, signer)
		case "creem":
			adapter, err = payments.NewCreem(payments.CreemConfig{Channel: channel, APIKey: apiKey, ProductID: c.ProductID, ProductAmountMinor: c.ProductAmountMinor, SuccessURL: c.SuccessURL})
		case "coinbase":
			adapter, err = payments.NewCoinbase(payments.CoinbaseConfig{Channel: channel, APIKey: apiKey, PrivateKey: privateKey, SuccessURL: c.SuccessURL, CancelURL: c.CancelURL})
		case "waffo":
			adapter, err = payments.NewWaffo(payments.WaffoConfig{Channel: channel, APIKey: apiKey, PrivateKey: privateKey, NotifyURL: c.NotifyURL, ReturnURL: c.ReturnURL, PayMethodType: c.PayMethodType, PayMethodName: c.PayMethodName})
		case "waffo_pancake":
			adapter, err = payments.NewPancake(payments.PancakeConfig{Channel: channel, PrivateKey: privateKey, StoreID: c.StoreID, ProductID: c.ProductID, ReturnURL: c.ReturnURL, CheckoutRegion: c.CheckoutRegion, CheckoutLanguage: c.CheckoutLanguage, TaxCategory: c.TaxCategory})
		}
		if err != nil {
			return nil, errors.New("invalid payment channel configuration")
		}
		adapters = append(adapters, adapter)
	}
	return adapters, nil
}
