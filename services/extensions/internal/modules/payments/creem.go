// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

// Creem sells a configured one-time product. Its checkout API does not accept
// an arbitrary amount override; the native wallet exposes this fixed amount.
type CreemConfig struct {
	Channel            Channel
	APIKey             string
	ProductID          string
	ProductAmountMinor int64
	SuccessURL         string
}

type Creem struct {
	cfg    CreemConfig
	client *http.Client
}

func NewCreem(cfg CreemConfig) (*Creem, error) {
	if !cfg.Channel.valid() || cfg.Channel.Provider != "creem" || (cfg.Channel.Currency != "USD" && cfg.Channel.Currency != "CNY" && cfg.Channel.Currency != "EUR") ||
		!validID(cfg.ProductID) || !validAmount(cfg.ProductAmountMinor) || !validProviderCredential(cfg.APIKey) {
		return nil, ErrInvalid
	}
	if _, err := secureURL(cfg.SuccessURL); err != nil {
		return nil, err
	}
	return &Creem{cfg: cfg, client: outboundClient()}, nil
}

func (c *Creem) Channel() Channel        { return c.cfg.Channel }
func (c *Creem) FixedAmountMinor() int64 { return c.cfg.ProductAmountMinor }
func (c *Creem) channelBinding() string  { return c.cfg.ProductID }
func (*Creem) RefundEnabled() bool       { return false }
func (*Creem) PartialRefunds() bool      { return false }

// The provider documents request_id deduplication, but no retention window.
// Preserve an ambiguous operation for its callback instead of guessing one.
func (*Creem) ReplayWindow(string) time.Duration { return 0 }
func (*Creem) Refund(context.Context, Order, Refund, string) (RefundResult, error) {
	return RefundResult{}, ErrUnsupported
}

func (c *Creem) api(ctx context.Context, method, path string, value any) ([]byte, error) {
	origin := "https://api.creem.io"
	if c.cfg.Channel.Environment == "test" {
		origin = "https://test-api.creem.io"
	}
	return requestJSONObject(ctx, c.client, method, origin+"/v1/"+path, value, http.Header{"X-Api-Key": {c.cfg.APIKey}})
}

func (c *Creem) Checkout(ctx context.Context, o Order, _ string) (Checkout, error) {
	if !providerOrderMatches(o, c.cfg.Channel) || o.Intent.AmountMinor != c.cfg.ProductAmountMinor {
		return Checkout{}, ErrInvalid
	}
	// A changed catalogue price or tax added at checkout would charge a different
	// amount from the user's frozen quote. Reject it before creating a session.
	productRaw, err := c.api(ctx, http.MethodGet, "products?product_id="+url.QueryEscape(c.cfg.ProductID), nil)
	if err != nil {
		return Checkout{}, err
	}
	var product struct {
		ID          string `json:"id"`
		Object      string `json:"object"`
		Mode        string `json:"mode"`
		Price       int64  `json:"price"`
		Currency    string `json:"currency"`
		BillingType string `json:"billing_type"`
		Status      string `json:"status"`
		TaxMode     string `json:"tax_mode"`
	}
	if jsonObject(productRaw, &product) != nil || product.ID != c.cfg.ProductID || product.Object != "product" || product.Mode != c.mode() ||
		product.Price != o.Intent.AmountMinor || strings.ToUpper(product.Currency) != o.Intent.Currency || product.BillingType != "onetime" || product.Status != "active" || product.TaxMode != "inclusive" {
		return Checkout{}, ErrConflict
	}
	raw, err := c.api(ctx, http.MethodPost, "checkouts", map[string]any{
		"product_id": c.cfg.ProductID, "request_id": o.ID, "units": 1,
		"success_url": c.cfg.SuccessURL,
		"metadata":    map[string]string{"lmm_order_id": o.ID, "lmm_core_intent_id": strconv.FormatInt(o.Intent.ID, 10), "lmm_merchant": o.Intent.Merchant, "lmm_product_id": c.cfg.ProductID},
	})
	if err != nil {
		return Checkout{}, err
	}
	var v creemCheckout
	if jsonObject(raw, &v) != nil || !validID(v.ID) || v.Object != "checkout" || v.RequestID != o.ID || v.Mode != c.mode() || refID(v.Product) != c.cfg.ProductID {
		return Checkout{}, ErrConflict
	}
	u, err := secureURL(v.URL)
	if err != nil || (u.Hostname() != "checkout.creem.io" && u.Hostname() != "creem.io" && u.Hostname() != "www.creem.io" && u.Hostname() != "test-checkout.creem.io") {
		return Checkout{}, ErrConflict
	}
	return Checkout{ID: v.ID, URL: v.URL}, nil
}

type creemCheckout struct {
	ID        string            `json:"id"`
	Object    string            `json:"object"`
	RequestID string            `json:"request_id"`
	Mode      string            `json:"mode"`
	URL       string            `json:"checkout_url"`
	Status    string            `json:"status"`
	Product   json.RawMessage   `json:"product"`
	Metadata  map[string]string `json:"metadata"`
	Order     struct {
		Transaction string `json:"transaction"`
		Status      string `json:"status"`
		AmountPaid  int64  `json:"amount_paid"`
		Currency    string `json:"currency"`
		Mode        string `json:"mode"`
		Type        string `json:"type"`
		Product     string `json:"product"`
	} `json:"order"`
}

func (c *Creem) mode() string {
	if c.cfg.Channel.Environment == "live" {
		return "prod"
	}
	return "test"
}

// Verify only decodes bounded observations. Service.Receive submits the exact
// original bytes and signature to Core before acknowledging or posting credit.
func (c *Creem) Verify(r *http.Request, _ time.Time) (Event, error) {
	raw, signature, err := callbackBytes(r, "Creem-Signature")
	if err != nil {
		return Event{}, err
	}
	var v struct {
		ID     string        `json:"id"`
		Type   string        `json:"eventType"`
		Object creemCheckout `json:"object"`
	}
	if jsonObject(raw, &v) != nil || !validID(v.ID) {
		return Event{}, ErrInvalid
	}
	if v.Type != "checkout.completed" {
		return Event{}, ErrUnsupported
	}
	e, err := c.payment(v.Object)
	if err != nil {
		return Event{}, err
	}
	e.Evidence.EventID, e.Evidence.Payload, e.Evidence.Signature, e.Evidence.Source = v.ID, raw, signature, "webhook"
	return e, nil
}

func (c *Creem) payment(v creemCheckout) (Event, error) {
	if v.Object != "checkout" || v.Status != "completed" || v.Mode != c.mode() || v.Order.Mode != c.mode() || v.Order.Status != "paid" || refID(v.Product) != c.cfg.ProductID ||
		v.Metadata["lmm_merchant"] != c.cfg.Channel.Merchant || v.Order.Type != "" && v.Order.Type != "onetime" || v.Order.Product != "" && v.Order.Product != c.cfg.ProductID ||
		!validTopupOrderID(v.RequestID) || !validID(v.Order.Transaction) || v.Order.AmountPaid != c.cfg.ProductAmountMinor || strings.ToUpper(v.Order.Currency) != c.cfg.Channel.Currency {
		return Event{}, ErrConflict
	}
	ch := c.cfg.Channel
	return Event{OrderID: v.RequestID, Type: Paid, Currency: ch.Currency, AmountMinor: v.Order.AmountPaid,
		Evidence: Evidence{Provider: ch.Provider, Merchant: ch.Merchant, Environment: ch.Environment, Transaction: v.Order.Transaction}}, nil
}

func (c *Creem) Lookup(ctx context.Context, o Order) ([]Event, error) {
	if !providerOrderMatches(o, c.cfg.Channel) || !validID(o.Checkout.ID) {
		return nil, ErrUnsupported
	}
	raw, err := c.api(ctx, http.MethodGet, "checkouts?checkout_id="+url.QueryEscape(o.Checkout.ID), nil)
	if err != nil {
		return nil, err
	}
	var v creemCheckout
	if jsonObject(raw, &v) != nil || v.ID != o.Checkout.ID || v.RequestID != o.ID || v.Mode != c.mode() || refID(v.Product) != c.cfg.ProductID {
		return nil, ErrConflict
	}
	if v.Status != "completed" {
		return nil, nil
	}
	e, err := c.payment(v)
	if err != nil || e.AmountMinor != o.Intent.AmountMinor {
		return nil, ErrConflict
	}
	e.Evidence.Payload = raw
	apiEvidence(&e, v.Order.Transaction, Paid)
	return []Event{e}, nil
}
