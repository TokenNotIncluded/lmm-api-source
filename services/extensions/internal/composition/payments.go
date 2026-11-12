// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments/rustbridge"
)

type paymentDocument struct {
	Version  int              `toml:"version"`
	Provider string           `toml:"provider"`
	Channels []paymentChannel `toml:"channels"`
}
type paymentChannel struct {
	ID                       int64  `toml:"id"`
	Merchant                 string `toml:"merchant"`
	Enabled                  bool   `toml:"enabled"`
	SubscriptionEnabled      bool   `toml:"subscription_enabled"`
	APIKeyFile               string `toml:"api_key_file"`
	APIVersion               string `toml:"api_version"`
	SuccessURL               string `toml:"success_url"`
	CancelURL                string `toml:"cancel_url"`
	IsolatedTestMode         bool   `toml:"isolated_test_mode"`
	IsolatedProviderURL      string `toml:"isolated_provider_url"`
	IsolatedCheckoutOrigin   string `toml:"isolated_checkout_origin"`
	IsolatedReturnOrigin     string `toml:"isolated_return_origin"`
	IsolatedSecretDirectory  string `toml:"isolated_secret_directory"`
	IsolatedBindingTokenFile string `toml:"isolated_binding_token_file"`
}

func paymentConfigured(cfg Config) bool {
	return cfg.PaymentTokenFile != "" || cfg.Payments.URLFile != "" || cfg.Payments.Database != "" || cfg.PaymentConfigFile != ""
}

// Reject ignored financial configuration before New's empty-selection return.
func validatePaymentSelection(cfg Config) error {
	selected := false
	for _, name := range cfg.Selected {
		selected = selected || name == "payments"
	}
	if !selected {
		if paymentConfigured(cfg) {
			return errors.New("payments configuration requires explicit module selection")
		}
		return nil
	}
	if cfg.PaymentTokenFile == "" || cfg.PaymentConfigFile == "" {
		return errors.New("payments requires independent RPC and provider configuration")
	}
	if cfg.Payments.Component != "payments" || cfg.Payments.validate() != nil {
		return errors.New("payments requires dedicated storage")
	}
	return nil
}

// Stripe direct-account channels use the Core's fixed USD policy. This private
// document has no webhook secret or exchange-rate field. The isolated lab is
// a separate provider selection with a bound numeric-loopback capability.
func loadPaymentAdapters(path string, runtimeCredentials ...[]byte) ([]payments.Adapter, error) {
	return loadPaymentAdaptersForHost(context.Background(), path, "", false, runtimeCredentials...)
}
func loadPaymentAdaptersForHost(ctx context.Context, path, hostOrigin string, localHTTP bool, runtimeCredentials ...[]byte) ([]payments.Adapter, error) {
	return loadPaymentAdaptersWithSigner(ctx, path, hostOrigin, localHTTP, nil, runtimeCredentials...)
}

func loadPaymentAdaptersWithSigner(ctx context.Context, path, hostOrigin string, localHTTP bool, signer payments.EPayCheckoutSigner, runtimeCredentials ...[]byte) ([]payments.Adapter, error) {
	return loadPaymentAdaptersWithAdmission(ctx, path, hostOrigin, localHTTP, signer, nil, runtimeCredentials...)
}

// Admission is extracted from the same private document as outbound adapters.
// An omitted subscription_enabled is false, independently of wallet enabled.
func loadPaymentAdaptersWithAdmission(ctx context.Context, path, hostOrigin string, localHTTP bool, signer payments.EPayCheckoutSigner, admission map[int64]bool, runtimeCredentials ...[]byte) ([]payments.Adapter, error) {
	if !cleanPaymentPath(path) {
		return nil, errors.New("invalid private payment configuration path")
	}
	raw, err := coreclient.ReadPrivateFile(path, 64<<10)
	if err != nil || !utf8.Valid(raw) {
		return nil, errors.New("private payment configuration unavailable")
	}
	var version struct {
		Version int `toml:"version"`
	}
	if _, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&version); err != nil {
		return nil, errors.New("invalid private payment configuration")
	}
	if version.Version == 2 {
		return loadMultiPaymentAdaptersWithAdmission(raw, signer, admission)
	}
	var cfg paymentDocument
	meta, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&cfg)
	if err != nil || len(meta.Undecoded()) != 0 {
		return nil, errors.New("invalid private payment configuration")
	}
	allowed := map[string]bool{"version": true, "provider": true, "channels": true, "channels.id": true, "channels.merchant": true, "channels.enabled": true, "channels.subscription_enabled": true, "channels.api_key_file": true, "channels.api_version": true, "channels.success_url": true, "channels.cancel_url": true, "channels.isolated_test_mode": true, "channels.isolated_provider_url": true, "channels.isolated_checkout_origin": true, "channels.isolated_return_origin": true, "channels.isolated_secret_directory": true, "channels.isolated_binding_token_file": true}
	for _, key := range meta.Keys() {
		if !allowed[key.String()] {
			return nil, errors.New("unknown private payment configuration field")
		}
	}
	if cfg.Version != 1 || (cfg.Provider != "stripe-test" && cfg.Provider != "stripe-live" && cfg.Provider != "stripe-isolated-test") || !meta.IsDefined("channels") || len(cfg.Channels) > 16 {
		return nil, errors.New("payments require explicit bounded Stripe direct USD channels")
	}
	if cfg.Provider != "stripe-isolated-test" {
		for _, key := range meta.Keys() {
			if strings.HasPrefix(key.String(), "channels.isolated_") {
				return nil, errors.New("isolated fields require explicit isolated payments provider")
			}
		}
	}
	adapters := make([]payments.Adapter, 0, len(cfg.Channels))
	ids, merchants := map[int64]bool{}, map[string]bool{}
	for _, c := range cfg.Channels {
		if !cleanPaymentPath(c.APIKeyFile) || ids[c.ID] || merchants[c.Merchant] {
			return nil, errors.New("invalid or duplicate payment channel")
		}
		rawKey, err := coreclient.ReadPrivateFile(c.APIKeyFile, 4096)
		if err != nil {
			return nil, errors.New("private payment API credential unavailable")
		}
		key := strings.TrimSpace(string(rawKey))
		if len(key) < 16 || len(key) > 4096 {
			return nil, errors.New("invalid private payment API credential")
		}
		for _, b := range []byte(key) {
			if b < 33 || b > 126 {
				return nil, errors.New("invalid private payment API credential")
			}
		}
		environment := "test"
		if cfg.Provider == "stripe-live" {
			environment = "live"
		}
		stripeCfg := payments.StripeConfig{Channel: payments.Channel{ID: c.ID, Provider: "stripe", Merchant: c.Merchant, Environment: environment, Currency: "USD", Enabled: c.Enabled}, APIKey: key, APIVersion: c.APIVersion, SuccessURL: c.SuccessURL, CancelURL: c.CancelURL}
		var adapter *payments.Stripe
		if cfg.Provider == "stripe-isolated-test" {
			if hostOrigin == "" || c.IsolatedReturnOrigin != hostOrigin || (!localHTTP && strings.HasPrefix(hostOrigin, "http:")) || !loadedPaymentHostCredential(runtimeCredentials) || !cleanPaymentPath(c.IsolatedSecretDirectory) || !isolatedPaymentAPIFile(c.APIKeyFile, c.IsolatedSecretDirectory) || c.APIKeyFile == c.IsolatedBindingTokenFile {
				return nil, errors.New("isolated payments require independent private lab credentials")
			}
			adapter, err = payments.NewStripeIsolated(ctx, payments.StripeIsolatedConfig{Stripe: stripeCfg, TestMode: c.IsolatedTestMode, ProviderURL: c.IsolatedProviderURL, CheckoutOrigin: c.IsolatedCheckoutOrigin, ReturnOrigin: c.IsolatedReturnOrigin, SecretDirectory: c.IsolatedSecretDirectory, BindingTokenFile: c.IsolatedBindingTokenFile, ForbiddenCredentials: runtimeCredentials})
		} else {
			if c.IsolatedTestMode || c.IsolatedProviderURL != "" || c.IsolatedCheckoutOrigin != "" || c.IsolatedReturnOrigin != "" || c.IsolatedSecretDirectory != "" || c.IsolatedBindingTokenFile != "" {
				return nil, errors.New("isolated fields require explicit isolated payments provider")
			}
			adapter, err = payments.NewStripe(stripeCfg)
		}
		if err != nil {
			return nil, errors.New("invalid Stripe channel configuration")
		}
		if admission != nil {
			admission[c.ID] = c.SubscriptionEnabled
		}
		ids[c.ID], merchants[c.Merchant] = true, true
		adapters = append(adapters, adapter)
	}
	return adapters, nil
}
func cleanPaymentPath(path string) bool {
	return path != "" && filepath.IsAbs(path) && filepath.Clean(path) == path && !strings.ContainsAny(path, "\x00\r\n")
}

// The bytes checked here are the bytes passed to the actual clients. Each
// financial credential is read once, so atomic key-file rotation cannot swap
// a different credential between the isolation check and client construction.
func loadPaymentCredential(cfg Config, identityToken, commerceToken []byte) ([]byte, error) {
	paymentToken, err := coreclient.ReadTokenFile(cfg.PaymentTokenFile)
	if err != nil {
		return nil, errors.New("private payment RPC credential unavailable")
	}
	if len(cfg.HostCredential) < 32 {
		return nil, errors.New("payment RPC requires an independent loaded host credential")
	}
	if err := independentRuntimeCredentials(cfg.HostCredential, identityToken, commerceToken, paymentToken); err != nil {
		return nil, err
	}
	return paymentToken, nil
}

// Empty optional roles were not loaded and grant no authority. For actual app
// startup, Run always supplies its already validated host credential. Legacy
// internal composition callers may omit it when payments are not selected.
func independentRuntimeCredentials(credentials ...[]byte) error {
	for i, token := range credentials {
		if len(token) == 0 {
			continue
		}
		for _, other := range credentials[i+1:] {
			if len(other) != 0 && bytes.Equal(token, other) {
				return errors.New("host and core RPC credentials must be independent")
			}
		}
	}
	return nil
}
func (r *Runtime) composePayments(ctx context.Context, cfg Config, paymentToken []byte, otherCredentials ...[]byte) error {
	credentials := append([][]byte{cfg.HostCredential, paymentToken}, otherCredentials...)
	credentials = append(credentials, r.paymentIsolationCredentials...)
	defer func() { r.paymentIsolationCredentials = nil }()
	var err error
	r.PaymentClient, err = coreclient.New(cfg.Socket, paymentToken)
	if err != nil {
		return err
	}
	r.close = append(r.close, r.PaymentClient.Close)
	bridge, err := rustbridge.New(r.PaymentClient)
	if err != nil {
		return err
	}
	stripeAdmission := map[int64]bool{}
	adapters, err := loadPaymentAdaptersWithAdmission(ctx, cfg.PaymentConfigFile, cfg.HostPublicOrigin, cfg.LocalHTTP, bridge, stripeAdmission, credentials...)
	if err != nil {
		return err
	}
	db, err := openStorage(ctx, cfg.Payments)
	if err != nil {
		return err
	}
	r.close = append(r.close, db.Close)
	bounded, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	repo, err := payments.NewPostgres(bounded, db, cfg.Payments.Database)
	if err != nil {
		return errors.New("payment storage contract unavailable")
	}
	r.Payments, err = payments.New(repo, bridge, adapters...)
	if err != nil {
		return err
	}
	var pancake []*payments.Pancake
	var stripe []*payments.Stripe
	var epay []*payments.EPay
	for _, adapter := range adapters {
		if channel, ok := adapter.(*payments.Pancake); ok {
			pancake = append(pancake, channel)
		}
		if channel, ok := adapter.(*payments.Stripe); ok {
			stripe = append(stripe, channel)
		}
		if channel, ok := adapter.(*payments.EPay); ok {
			epay = append(epay, channel)
		}
	}
	if len(epay) > 0 {
		r.SubscriptionPayments, err = payments.NewEPaySubscriptions(r.PaymentClient, repo, epay)
	}
	if err != nil {
		return err
	}
	if len(pancake) > 0 {
		r.PancakeSubscriptionPayments, err = payments.NewPancakeSubscriptions(r.PaymentClient, db, pancake, stripeAdmission)
		if err != nil {
			return err
		}
	}
	if r.PancakeSubscriptionPayments != nil {
		r.PancakeSubscriptionPayments.AttachWallet(r.Payments)
	}
	stripeInstalled, e := paymentStorageAddonInstalled(ctx, db, []string{"public.payment_stripe_subscription_checkouts", "public.payment_stripe_subscription_inbox"})
	if e != nil {
		return e
	}
	stripeRequested := false
	for _, a := range stripe {
		stripeRequested = stripeRequested || stripeAdmission[a.Channel().ID]
	}
	if stripeRequested && !stripeInstalled {
		return errors.New("Stripe subscription admission requires explicit storage upgrade")
	}
	if len(stripe) > 0 && stripeInstalled {
		r.StripeSubscriptionPayments, err = payments.NewStripeSubscriptions(r.PaymentClient, db, stripe, stripeAdmission)
	}
	if err == nil && len(pancake) > 0 {
		r.PancakeAdmin, err = newPancakeAdministration(cfg.PaymentConfigFile, r.Payments, r.PancakeSubscriptionPayments)
	}
	return err
}

// Keep subscription orders behind the same host authentication and admission
// as wallet payments, while Core independently authorizes each current user.
type paymentModule struct {
	wallet        *payments.Service
	subscriptions *payments.EPaySubscriptions
	stripe        *payments.StripeSubscriptions
	pancake       *payments.PancakeSubscriptions
}

func (paymentModule) Name() string { return "payments" }
func (m paymentModule) Handler() http.Handler {
	mux := http.NewServeMux()
	if h := subscriptionPaymentHandler(m.subscriptions, m.stripe, m.pancake); h != nil {
		mux.Handle("/subscriptions/", http.StripPrefix("/subscriptions", h))
	}
	mux.Handle("/", m.wallet.Handler())
	return mux
}

func loadedPaymentHostCredential(credentials [][]byte) bool {
	return len(credentials) > 0 && len(credentials[0]) >= 32
}

func isolatedPaymentAPIFile(path, dir string) bool {
	if filepath.Dir(path) != dir {
		return false
	}
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
		return false
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	return ok && stat.Uid == uint32(os.Geteuid()) && stat.Nlink == 1
}

// SubscriptionPaymentHandler is also mounted by the existing Web user Session
// wrapper. Callback ingress is mounted separately with host admission.
func (r *Runtime) SubscriptionPaymentHandler() http.Handler {
	return subscriptionPaymentHandler(r.SubscriptionPayments, r.StripeSubscriptionPayments, r.PancakeSubscriptionPayments)
}
func subscriptionPaymentHandler(epay *payments.EPaySubscriptions, stripe *payments.StripeSubscriptions, pancake *payments.PancakeSubscriptions) http.Handler {
	if epay == nil && stripe == nil && pancake == nil {
		return nil
	}
	mux := http.NewServeMux()
	if pancake != nil {
		mux.Handle("/pancake/", http.StripPrefix("/pancake", pancake.Handler()))
	}
	if stripe != nil {
		mux.Handle("/stripe/", http.StripPrefix("/stripe", stripe.Handler()))
	}
	if epay != nil {
		mux.Handle("/", epay.Handler())
	}
	return mux
}
