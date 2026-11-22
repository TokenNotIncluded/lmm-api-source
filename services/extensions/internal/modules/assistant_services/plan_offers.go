package assistant_services

import (
	"context"
	"encoding/json"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/planoffers"
)

type PlanOffersClient interface {
	PlanOffersReady(context.Context) error
	GetAssistantPlanOffers(context.Context, string) (planoffers.Facts, error)
}
type CheckoutReader interface {
	CheckoutCatalogue(context.Context) planoffers.CheckoutObservation
}
type PlanOffersReader interface {
	PlanOffers(context.Context, string) (json.RawMessage, error)
}
type CorePlanOffers struct {
	client   PlanOffersClient
	auth     access.Authority
	checkout CheckoutReader
}

func NewCorePlanOffers(client PlanOffersClient, auth access.Authority, checkout CheckoutReader) (*CorePlanOffers, error) {
	if access.Missing(client) || access.Missing(auth) || access.Missing(checkout) {
		return nil, access.ErrUnavailable
	}
	return &CorePlanOffers{client: client, auth: auth, checkout: checkout}, nil
}
func (r *CorePlanOffers) Ready(ctx context.Context) error { return r.client.PlanOffersReady(ctx) }
func (r *CorePlanOffers) PlanOffers(ctx context.Context, credential string) (json.RawMessage, error) {
	actor, err := r.auth.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return nil, err
	}
	if actor.UserID <= 0 || actor.Account != (access.Account{Kind: "personal", ID: actor.UserID}) {
		return nil, access.ErrForbidden
	}
	facts, err := r.client.GetAssistantPlanOffers(ctx, credential)
	if err != nil {
		return nil, access.Translate(err)
	}
	if !facts.Valid(actor.UserID) {
		return nil, access.ErrUnavailable
	}
	checkout := r.checkout.CheckoutCatalogue(ctx)
	catalogue := checkout.Channels
	// This is an internal exact binding comparison, not a provider-health probe.
	find := func(id int64, binding string, subscription bool) (planoffers.CheckoutChannel, bool) {
		for _, c := range catalogue {
			if c.ChannelID == id && c.Binding == binding && c.Subscription == subscription {
				return c, true
			}
		}
		return planoffers.CheckoutChannel{}, false
	}
	plans := []any{}
	channels := []any{}
	discounts := []any{}
	available := false
	for _, p := range facts.Plans {
		limitReached := p.Terms.MaxPurchases > 0 && *p.Purchases >= p.Terms.MaxPurchases
		prices := []any{}
		for _, price := range p.ExternalPrices {
			_, host := find(price.ChannelID, price.Binding, true)
			usable := *price.Configured && host && checkout.PaymentRPCVerified && !limitReached
			reason := ""
			if !*price.Configured {
				reason = "core_checkout_not_configured"
			} else if !host {
				reason = "host_subscription_not_enabled_or_binding_changed"
			} else if !checkout.PaymentRPCVerified {
				reason = "payment_service_not_verified"
			} else if limitReached {
				reason = "purchase_limit_reached"
			}
			available = available || usable
			prices = append(prices, map[string]any{"channel_id": price.ChannelID,
				"provider": price.Provider, "environment": price.Environment, "currency": price.Currency,
				"amount_minor": price.AmountMinor, "checkout_available": usable, "reason": reason})
		}
		plans = append(plans, map[string]any{"id": p.ID, "revision": p.Revision, "terms": p.Terms,
			"purchases": *p.Purchases, "purchase_limit_reached": limitReached,
			"wallet_payment_allowed": p.Terms.WalletAllowed() && !limitReached,
			"wallet_balance_checked": false, "external_prices": prices})
	}
	for _, c := range facts.TopupChannels {
		host, matched := find(c.ChannelID, c.Binding, false)
		lo, _ := strconv.ParseInt(c.MinAmountMinor, 10, 64)
		hi, _ := strconv.ParseInt(c.MaxAmountMinor, 10, 64)
		fixedValid := host.FixedAmountMinor == 0 || (host.FixedAmountMinor >= lo && host.FixedAmountMinor <= hi)
		usable := *c.Configured && matched && checkout.PaymentRPCVerified && fixedValid && !*facts.TopupRestricted
		reason := ""
		if *facts.TopupRestricted {
			reason = "new_topup_restricted"
		} else if !*c.Configured {
			reason = "core_topup_not_configured"
		} else if !matched {
			reason = "host_topup_not_enabled_or_binding_changed"
		} else if !checkout.PaymentRPCVerified {
			reason = "payment_service_not_verified"
		} else if !fixedValid {
			reason = "fixed_product_amount_outside_core_bounds"
		}
		available = available || usable
		channel := map[string]any{"channel_id": c.ChannelID, "provider": c.Provider, "environment": c.Environment,
			"currency": c.Currency, "payment_type": c.PaymentType, "checkout_available": usable, "reason": reason,
			"min_amount_minor": c.MinAmountMinor, "max_amount_minor": c.MaxAmountMinor,
			"credits_per_minor": c.CreditsPerMinor}
		if matched && host.FixedAmountMinor > 0 {
			channel["fixed_amount_minor"] = host.FixedAmountMinor
		}
		channels = append(channels, channel)
		if usable && c.CreditQuote != nil {
			discounts = append(discounts, map[string]any{"channel_id": c.ChannelID, "currency": c.Currency,
				"match": "exact_credit_quantity", "credit_unit": "native_credit_500k_per_usd",
				"settlement_per_usd_numerator":   c.CreditQuote.SettlementNumerator,
				"settlement_per_usd_denominator": c.CreditQuote.SettlementDenominator,
				"discounts":                      c.CreditQuote.Discounts})
		}
	}
	current, err := r.auth.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return nil, err
	}
	if current != actor {
		return nil, access.ErrConflict
	}
	raw, err := json.Marshal(map[string]any{
		"ok": true, "scope": "self", "checked_at_unix": facts.CheckedAt,
		"checkout_available": available, "read_only": !available,
		"new_topup_restricted": *facts.TopupRestricted, "plans": plans,
		"topup_channels": channels, "topup_discounts": discounts,
		"observation":             "current_native_configuration_and_live_session",
		"provider_health_checked": false, "order_created": false,
		"payment_rpc_verified": checkout.PaymentRPCVerified,
		"message":              "Availability reflects the current Core policy, configured Web services and authenticated payment RPC. A fresh checkout still validates the order, limits, balance and provider response.",
	})
	if err != nil || len(raw) > 48<<10 {
		return nil, access.ErrUnavailable
	}
	return raw, nil
}
func planOfferDefinitions() []assistantOpenAIToolDefinition {
	return []assistantOpenAIToolDefinition{{Type: "function", Function: assistantOpenAIToolFunction{
		Name:        "get_plan_offers",
		Description: "Read currently published subscription plans, exact per-channel top-up discounts, and the signed-in user's current checkout eligibility. Availability requires the live Core policy, an actually configured Web payment service and its authenticated payment RPC; provider health is not probed. This never creates an order, reserves credit, purchases a plan, or reveals payment credentials. A published price alone does not make checkout available.",
		Parameters:  emptyObjectSchema(),
	}}}
}
