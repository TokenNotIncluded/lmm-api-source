// Package planoffers contains the closed Core/extension read contract. Values
// describe configuration observed now; none authorize or reserve a payment.
package planoffers

import (
	"encoding/hex"
	"strconv"
	"strings"
	"unicode"
)

const Contract = "assistant.plan-offers.v1"
const Feature = "assistant.plan_offers.v1"

type Facts struct {
	Contract        string    `json:"contract"`
	ActorUserID     int64     `json:"actor_user_id"`
	CheckedAt       int64     `json:"checked_at_unix"`
	TopupRestricted *bool     `json:"topup_restricted"`
	Plans           []Plan    `json:"plans"`
	TopupChannels   []Channel `json:"topup_channels"`
}
type Terms struct {
	Title           string   `json:"title"`
	PriceUnits      string   `json:"price_units"`
	DurationSeconds int64    `json:"duration_seconds"`
	DurationMonths  int64    `json:"duration_months"`
	ResetPeriod     string   `json:"reset_period"`
	ResetSeconds    int64    `json:"reset_seconds"`
	LimitUnits      string   `json:"limit_units"`
	Models          []string `json:"models"`
	Groups          []string `json:"groups"`
	MaxPurchases    int64    `json:"max_purchases_per_user"`
	Enabled         *bool    `json:"enabled"`
	// Core deliberately omits true to preserve its original PlanTerms contract.
	AllowBalancePay *bool `json:"allow_balance_pay,omitempty"`
}

func (t Terms) WalletAllowed() bool { return t.AllowBalancePay == nil || *t.AllowBalancePay }

type Plan struct {
	ID             int64   `json:"id"`
	Revision       int64   `json:"revision"`
	Terms          Terms   `json:"terms"`
	Purchases      *int64  `json:"purchases"`
	ExternalPrices []Price `json:"external_prices"`
}
type Price struct {
	ChannelID   int64  `json:"channel_id"`
	Provider    string `json:"provider"`
	Environment string `json:"environment"`
	Currency    string `json:"currency"`
	AmountMinor string `json:"amount_minor"`
	Binding     string `json:"binding_sha256"`
	Configured  *bool  `json:"configured"`
}
type Ratio struct {
	Numerator   int64 `json:"numerator"`
	Denominator int64 `json:"denominator"`
}
type CreditQuote struct {
	MinCreditUnits        *int64           `json:"min_credit_units,omitempty"`
	SettlementNumerator   int64            `json:"settlement_per_usd_numerator"`
	SettlementDenominator int64            `json:"settlement_per_usd_denominator"`
	Discounts             map[string]Ratio `json:"discounts"`
}

// Older Core facts omitted the optional policy and admitted any positive credit
// quantity. This is a wire default, not an extension-owned payment policy.
func (q CreditQuote) MinimumCreditUnits() int64 {
	if q.MinCreditUnits == nil {
		return 1
	}
	return *q.MinCreditUnits
}

type Channel struct {
	ChannelID       int64        `json:"channel_id"`
	Provider        string       `json:"provider"`
	Environment     string       `json:"environment"`
	Currency        string       `json:"currency"`
	PaymentType     string       `json:"payment_type"`
	Binding         string       `json:"binding_sha256"`
	Configured      *bool        `json:"configured"`
	MinAmountMinor  string       `json:"min_amount_minor"`
	MaxAmountMinor  string       `json:"max_amount_minor"`
	CreditsPerMinor string       `json:"credits_per_minor"`
	CreditQuote     *CreditQuote `json:"credit_quote"`
}

// CheckoutChannel comes only from constructed Go payment services and their
// current independent admission switches. Binding is never returned to a model.
type CheckoutObservation struct {
	PaymentRPCVerified bool
	Channels           []CheckoutChannel
}

type CheckoutChannel struct {
	ChannelID        int64
	Binding          string
	Subscription     bool
	FixedAmountMinor int64
}

func number(s string, min, max int64) bool {
	n, err := strconv.ParseInt(s, 10, 64)
	return err == nil && n >= min && n <= max && strconv.FormatInt(n, 10) == s
}
func label(s string, max int) bool {
	return s != "" && len(s) <= max && strings.IndexFunc(s, unicode.IsControl) < 0
}
func digest(s string) bool {
	b, err := hex.DecodeString(s)
	return err == nil && len(b) == 32 && hex.EncodeToString(b) == s
}
func provider(p string) bool {
	switch p {
	case "epay", "stripe", "creem", "waffo", "waffo_pancake", "coinbase":
		return true
	}
	return false
}
func environment(s string) bool { return s == "live" || s == "test" }
func (f Facts) Valid(user int64) bool {
	if f.Contract != Contract || user <= 0 || f.ActorUserID != user || f.CheckedAt <= 0 || f.TopupRestricted == nil ||
		f.Plans == nil || len(f.Plans) > 1000 || f.TopupChannels == nil || len(f.TopupChannels) > 256 {
		return false
	}
	ids := map[int64]bool{}
	for _, p := range f.Plans {
		if p.ID <= 0 || p.Revision <= 0 || ids[p.ID] || p.Purchases == nil || *p.Purchases < 0 ||
			p.ExternalPrices == nil || len(p.ExternalPrices) > 256 || !p.Terms.valid() {
			return false
		}
		ids[p.ID] = true
		channels := map[int64]bool{}
		for _, e := range p.ExternalPrices {
			if e.ChannelID <= 0 || channels[e.ChannelID] || !provider(e.Provider) || !environment(e.Environment) ||
				!label(e.Currency, 12) || !number(e.AmountMinor, 1, 9000000000000) || !digest(e.Binding) || e.Configured == nil {
				return false
			}
			channels[e.ChannelID] = true
		}
	}
	channels := map[int64]bool{}
	for _, c := range f.TopupChannels {
		if c.ChannelID <= 0 || channels[c.ChannelID] || !provider(c.Provider) || !environment(c.Environment) ||
			!label(c.Currency, 12) || !digest(c.Binding) || c.Configured == nil ||
			!number(c.MinAmountMinor, 0, 9000000000000) || !number(c.MaxAmountMinor, 1, 9000000000000) ||
			!number(c.CreditsPerMinor, 0, 9000000000000) {
			return false
		}
		lo, _ := strconv.ParseInt(c.MinAmountMinor, 10, 64)
		hi, _ := strconv.ParseInt(c.MaxAmountMinor, 10, 64)
		if lo > hi {
			return false
		}
		if c.CreditQuote == nil {
			if c.CreditsPerMinor == "0" {
				return false
			}
		} else {
			q := c.CreditQuote
			if q.MinimumCreditUnits() <= 0 || q.SettlementNumerator <= 0 || q.SettlementNumerator > 1000000000 || q.SettlementDenominator <= 0 ||
				q.SettlementDenominator > 1000000000 || q.Discounts == nil || len(q.Discounts) > 1000 ||
				q.SettlementNumerator > 5000*q.SettlementDenominator {
				return false
			}
			for quantity, r := range q.Discounts {
				if !number(quantity, 1, 9223372036854775807) || r.Numerator <= 0 || r.Numerator > r.Denominator ||
					r.Denominator <= 0 || r.Denominator > 1000000000 {
					return false
				}
			}
		}
		channels[c.ChannelID] = true
	}
	return true
}
func (t Terms) valid() bool {
	if !label(t.Title, 160) || !number(t.PriceUnits, 1, 1000000000000) ||
		!number(t.LimitUnits, 1, 1000000000000) || t.Enabled == nil || !*t.Enabled ||
		t.MaxPurchases < 0 || t.MaxPurchases > 10000 ||
		!((t.DurationMonths == 0 && t.DurationSeconds >= 60 && t.DurationSeconds <= 315576000) ||
			(t.DurationMonths >= 1 && t.DurationMonths <= 120 && t.DurationSeconds == 0)) {
		return false
	}
	switch t.ResetPeriod {
	case "grant", "day", "week", "month", "anniversary_month":
		if t.ResetSeconds != 0 {
			return false
		}
	case "custom":
		if t.ResetSeconds < 1 || t.ResetSeconds > 315576000 {
			return false
		}
	default:
		return false
	}
	for _, list := range [][]string{t.Models, t.Groups} {
		if len(list) == 0 || len(list) > 256 {
			return false
		}
		seen := map[string]bool{}
		for _, v := range list {
			if !label(v, 128) || seen[v] {
				return false
			}
			seen[v] = true
		}
	}
	return true
}
