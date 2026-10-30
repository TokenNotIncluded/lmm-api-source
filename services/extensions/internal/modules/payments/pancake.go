// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"net/http"
	"strconv"
	"time"

	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

type PancakeConfig struct {
	Channel          Channel
	PrivateKey       string
	StoreID          string
	ProductID        string
	ReturnURL        string
	CheckoutRegion   string
	CheckoutLanguage string
	TaxCategory      string
}

type Pancake struct {
	cfg    PancakeConfig
	client *pancake.Client
}

func NewPancake(cfg PancakeConfig) (*Pancake, error) {
	return newPancake(cfg, outboundClient())
}

func newPancake(cfg PancakeConfig, httpClient *http.Client) (*Pancake, error) {
	if !cfg.Channel.valid() || cfg.Channel.Provider != "waffo_pancake" || (cfg.Channel.Currency != "USD" && cfg.Channel.Currency != "CNY") || !validID(cfg.StoreID) || !validID(cfg.ProductID) {
		return nil, ErrInvalid
	}
	if cfg.CheckoutRegion != "" && cfg.CheckoutRegion != "global" && cfg.CheckoutRegion != "china" {
		return nil, ErrInvalid
	}
	if _, err := secureURL(cfg.ReturnURL); err != nil {
		return nil, err
	}
	switch pancake.TaxCategory(cfg.TaxCategory) {
	case pancake.TaxCategoryDigitalGoods, pancake.TaxCategorySaaS, pancake.TaxCategorySoftware,
		pancake.TaxCategoryEbook, pancake.TaxCategoryOnlineCourse, pancake.TaxCategoryConsulting, pancake.TaxCategoryProfessionalService:
	default:
		return nil, ErrInvalid
	}
	environment := pancake.EnvironmentTest
	if cfg.Channel.Environment == "live" {
		environment = pancake.EnvironmentProd
	}
	bounded := *httpClient
	bounded.Transport = boundedProviderTransport{transport: httpClient.Transport}
	client, err := pancake.New(pancake.Config{MerchantID: cfg.Channel.Merchant, PrivateKey: cfg.PrivateKey, Environment: environment, HTTPClient: &bounded})
	if err != nil {
		return nil, ErrInvalid
	}
	return &Pancake{cfg: cfg, client: client}, nil
}

func (p *Pancake) Channel() Channel       { return p.cfg.Channel }
func (p *Pancake) channelBinding() string { return p.cfg.StoreID + "/" + p.cfg.ProductID }
func (*Pancake) RefundEnabled() bool      { return false }
func (*Pancake) PartialRefunds() bool     { return false }
func (*Pancake) ReplayWindow(operation string) time.Duration {
	if operation == "checkout" {
		return 23 * time.Hour // SDK v0.17 explicitly documents a 24-hour key cache.
	}
	return 0
}
func (*Pancake) Refund(context.Context, Order, Refund, string) (RefundResult, error) {
	return RefundResult{}, ErrUnsupported
}

func (p *Pancake) Checkout(ctx context.Context, o Order, key string) (Checkout, error) {
	if !providerOrderMatches(o, p.cfg.Channel) || !validKey(key) {
		return Checkout{}, ErrInvalid
	}
	expires := 45 * 60
	params := pancake.AuthenticatedCheckoutParams{
		CreateCheckoutSessionParams: pancake.CreateCheckoutSessionParams{
			ProductID: p.cfg.ProductID, Currency: p.cfg.Channel.Currency,
			PriceSnapshot:           &pancake.PriceSnapshot{Amount: decimalMinor(o.Intent.AmountMinor), TaxCategory: pancake.TaxCategory(p.cfg.TaxCategory)},
			OrderMerchantExternalID: &o.ID, SuccessURL: &p.cfg.ReturnURL, ExpiresInSeconds: &expires,
			Metadata: map[string]string{"lmm_product_id": p.cfg.ProductID, "lmm_order_id": o.ID, "lmm_core_intent_id": strconv.FormatInt(o.Intent.ID, 10), "lmm_merchant": o.Intent.Merchant},
		},
		BuyerIdentity: "lmm-user-" + strconv.FormatInt(o.Intent.CreatedBy, 10),
	}
	if p.cfg.CheckoutRegion == "china" {
		params.BillingDetail = &pancake.BillingDetail{Country: "CN", IsBusiness: false}
	}
	if p.cfg.CheckoutLanguage != "" {
		language := pancake.CashierLanguage(p.cfg.CheckoutLanguage)
		params.Language = &language
	}
	session, err := p.client.Checkout.Authenticated.Create(ctx, params, pancake.WithIdempotencyKey(key))
	if err != nil {
		return Checkout{}, ErrUnavailable
	}
	if session == nil || !validID(session.SessionID) {
		return Checkout{}, ErrConflict
	}
	if _, err = secureURL(session.CheckoutURL); err != nil {
		return Checkout{}, ErrConflict
	}
	return Checkout{ID: session.SessionID, URL: session.CheckoutURL}, nil
}

func (p *Pancake) Verify(r *http.Request, _ time.Time) (Event, error) {
	raw, signature, err := callbackBytes(r, "X-Waffo-Signature")
	if err != nil {
		return Event{}, err
	}
	var v pancake.TypedWebhookEvent[pancake.WebhookEventData]
	if jsonObject(raw, &v) != nil || !validID(v.ID) {
		return Event{}, ErrInvalid
	}
	if v.EventType != "order.completed" {
		return Event{}, ErrUnsupported
	}
	mode := pancake.EnvironmentTest
	if p.cfg.Channel.Environment == "live" {
		mode = pancake.EnvironmentProd
	}
	d := v.Data
	if v.Mode != mode || v.StoreID != p.cfg.StoreID || d.OrderMetadata["lmm_product_id"] != p.cfg.ProductID || d.OrderMetadata["lmm_merchant"] != p.cfg.Channel.Merchant ||
		d.OrderMerchantExternalID == nil || !validTopupOrderID(*d.OrderMerchantExternalID) || d.Currency != p.cfg.Channel.Currency ||
		d.PaymentID == nil || !validID(*d.PaymentID) || d.OrderStatus != nil && *d.OrderStatus != "completed" || d.PaymentStatus != nil && *d.PaymentStatus != "succeeded" {
		return Event{}, ErrConflict
	}
	amount := d.Amount
	if d.ChargedAmount != nil {
		amount = *d.ChargedAmount
		if d.Amount != "" {
			legacy, legacyErr := minorDecimal(d.Amount)
			charged, chargedErr := minorDecimal(amount)
			if legacyErr != nil || chargedErr != nil || legacy != charged {
				return Event{}, ErrConflict
			}
		}
	} else if d.ListPrice != nil {
		// A catalogue snapshot does not prove that the payment channel charged it.
		return Event{}, ErrEvidence
	}
	minor, err := minorDecimal(amount)
	if err != nil {
		return Event{}, err
	}
	c := p.cfg.Channel
	return Event{OrderID: *d.OrderMerchantExternalID, Type: Paid, Currency: c.Currency, AmountMinor: minor,
		Evidence: Evidence{Provider: c.Provider, Merchant: c.Merchant, Environment: c.Environment, Transaction: *d.PaymentID, EventID: v.ID,
			Payload: raw, Signature: signature, Source: "webhook"}}, nil
}

func (p *Pancake) Lookup(ctx context.Context, o Order) ([]Event, error) {
	if !providerOrderMatches(o, p.cfg.Channel) {
		return nil, ErrInvalid
	}
	// SDK v0.17 documents this business-reference lookup. Its price snapshot
	// fields are not a charged-amount proof, so only a signed webhook can settle.
	result, err := p.client.GraphQL.Query(ctx, pancake.GraphQLParams{
		Query:     `query ($ref: String!) { payments(limit: 2, filter: { orderMerchantExternalId: { eq: $ref } }) { id status orderMerchantExternalId } }`,
		Variables: map[string]any{"ref": o.ID},
	})
	if err != nil || result == nil || len(result.Errors) != 0 {
		return nil, ErrUnavailable
	}
	var data struct {
		Payments []struct {
			ID    string `json:"id"`
			State string `json:"status"`
			Order string `json:"orderMerchantExternalId"`
		} `json:"payments"`
	}
	if jsonObject(result.Data, &data) != nil {
		return nil, ErrInvalid
	}
	for _, payment := range data.Payments {
		if payment.Order != o.ID || !validID(payment.ID) || o.PaymentID != "" && payment.ID != o.PaymentID {
			return nil, ErrConflict
		}
		if payment.State == "succeeded" {
			return nil, ErrEvidence
		}
	}
	return nil, nil
}
