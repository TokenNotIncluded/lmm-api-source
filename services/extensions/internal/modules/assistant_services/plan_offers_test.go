package assistant_services

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/planoffers"
)

func offersBool(v bool) *bool  { return &v }
func offersInt(v int64) *int64 { return &v }

type planOffersPort struct {
	facts      planoffers.Facts
	calls      int
	credential string
}

func (p *planOffersPort) PlanOffersReady(context.Context) error { return nil }
func (p *planOffersPort) GetAssistantPlanOffers(_ context.Context, c string) (planoffers.Facts, error) {
	p.calls++
	p.credential = c
	return p.facts, nil
}

type planOffersAuthority struct {
	calls  int
	change bool
	deny   bool
}

func (a *planOffersAuthority) Check(_ context.Context, _ string, account access.Account, permission access.Permission) (access.Principal, error) {
	a.calls++
	if permission != access.Read || account != (access.Account{}) {
		return access.Principal{}, access.ErrForbidden
	}
	if a.deny && a.calls > 1 {
		return access.Principal{}, access.ErrUnauthorized
	}
	user := int64(7)
	if a.change && a.calls > 1 {
		user = 8
	}
	return access.Principal{UserID: user, Account: access.Account{Kind: "personal", ID: user}}, nil
}

type offersCatalogue []planoffers.CheckoutChannel

func (c offersCatalogue) CheckoutCatalogue(context.Context) planoffers.CheckoutObservation {
	return planoffers.CheckoutObservation{PaymentRPCVerified: true, Channels: c}
}
func offersFixture() planoffers.Facts {
	return planoffers.Facts{Contract: planoffers.Contract, ActorUserID: 7, CheckedAt: 123456, TopupRestricted: offersBool(false),
		Plans: []planoffers.Plan{{ID: 1, Revision: 2, Purchases: offersInt(0), Terms: planoffers.Terms{
			Title: "Actual plan", PriceUnits: "100", DurationSeconds: 3600, ResetPeriod: "grant", LimitUnits: "1000",
			Models: []string{"*"}, Groups: []string{"*"}, Enabled: offersBool(true), MaxPurchases: 1},
			ExternalPrices: []planoffers.Price{{ChannelID: 7, Provider: "epay", Environment: "test", Currency: "CNY",
				AmountMinor: "125", Binding: strings.Repeat("a", 64), Configured: offersBool(true)}}}},
		TopupChannels: []planoffers.Channel{{ChannelID: 7, Provider: "epay", Environment: "test", Currency: "CNY", PaymentType: "alipay",
			Binding: strings.Repeat("b", 64), Configured: offersBool(true), MinAmountMinor: "1", MaxAmountMinor: "10000", CreditsPerMinor: "1",
			CreditQuote: &planoffers.CreditQuote{SettlementNumerator: 7, SettlementDenominator: 1, Discounts: map[string]planoffers.Ratio{
				"500000": {Numerator: 9, Denominator: 10}}}}},
	}
}
func readOffers(t *testing.T, f planoffers.Facts, c offersCatalogue) map[string]any {
	t.Helper()
	p := &planOffersPort{facts: f}
	a := &planOffersAuthority{}
	r, err := NewCorePlanOffers(p, a, c)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := r.PlanOffers(context.Background(), "synthetic-original-session")
	if err != nil {
		t.Fatal(err)
	}
	if a.calls != 2 || p.calls != 1 || p.credential != "synthetic-original-session" {
		t.Fatal(a.calls, p.calls)
	}
	if strings.Contains(string(raw), "binding_sha256") || strings.Contains(string(raw), strings.Repeat("a", 64)) || strings.Contains(string(raw), "synthetic-original-session") {
		t.Fatal("private binding leaked")
	}
	var v map[string]any
	if json.Unmarshal(raw, &v) != nil {
		t.Fatal("invalid output")
	}
	return v
}
func TestPlanOffersJoinsActualCheckoutAndIndependentAdmission(t *testing.T) {
	f := offersFixture()
	all := offersCatalogue{{ChannelID: 7, Binding: strings.Repeat("a", 64), Subscription: true}, {ChannelID: 7, Binding: strings.Repeat("b", 64)}}
	v := readOffers(t, f, all)
	if v["checkout_available"] != true || len(v["topup_discounts"].([]any)) != 1 || v["provider_health_checked"] != false || v["order_created"] != false {
		t.Fatal(v)
	}
	d := v["topup_discounts"].([]any)[0].(map[string]any)
	if d["match"] != "exact_credit_quantity" || d["currency"] != "CNY" || d["settlement_per_usd_numerator"] != float64(7) {
		t.Fatal(d)
	}
	// A published Core price cannot substitute for a host adapter, even at the
	// same channel ID. The binding includes the actual merchant and currency.
	wrong := offersCatalogue{{ChannelID: 7, Binding: strings.Repeat("c", 64), Subscription: true}}
	v = readOffers(t, f, wrong)
	if v["checkout_available"] != false || len(v["topup_discounts"].([]any)) != 0 {
		t.Fatal(v)
	}
	// Wallet off leaves the independently admitted subscription route visible.
	f.TopupChannels[0].Configured = offersBool(false)
	v = readOffers(t, f, all[:1])
	if v["checkout_available"] != true || v["topup_channels"].([]any)[0].(map[string]any)["checkout_available"] != false {
		t.Fatal(v)
	}
}
func TestPlanOffersMarkedTopupAndPurchaseLimitsRemainDistinct(t *testing.T) {
	f := offersFixture()
	f.TopupRestricted = offersBool(true)
	c := offersCatalogue{{ChannelID: 7, Binding: strings.Repeat("a", 64), Subscription: true}, {ChannelID: 7, Binding: strings.Repeat("b", 64)}}
	v := readOffers(t, f, c)
	if v["checkout_available"] != true || len(v["topup_discounts"].([]any)) != 0 {
		t.Fatal(v)
	}
	f.Plans[0].Purchases = offersInt(1)
	v = readOffers(t, f, c)
	p := v["plans"].([]any)[0].(map[string]any)
	if v["checkout_available"] != false || p["purchase_limit_reached"] != true || p["wallet_payment_allowed"] != false {
		t.Fatal(v)
	}
}
func TestPlanOffersRevocationAndActorDriftRejectReadback(t *testing.T) {
	for _, change := range []bool{false, true} {
		p := &planOffersPort{facts: offersFixture()}
		a := &planOffersAuthority{change: change, deny: !change}
		r, _ := NewCorePlanOffers(p, a, offersCatalogue{})
		if _, err := r.PlanOffers(context.Background(), "original"); err == nil {
			t.Fatal("accepted changed Session")
		}
	}
}
func TestPlanOffersInstalledBindingRejectsModelAuthority(t *testing.T) {
	service := &Service{}
	present := func(list []assistant.AgentToolBinding) bool {
		for _, b := range list {
			if b.Name == "get_plan_offers" {
				return true
			}
		}
		return false
	}
	if !present(service.Bindings()) || present(service.AvailableBindings()) {
		t.Fatal("unconfigured tool advertised")
	}
	p := &planOffersPort{facts: offersFixture()}
	r, _ := NewCorePlanOffers(p, &planOffersAuthority{}, offersCatalogue{})
	service.dependencies.PlanOffers = r
	if !present(service.AvailableBindings()) {
		t.Fatal("complete reader not advertised")
	}
	var in assistant.AgentInvocation
	in.Credential = "original"
	in.Call.Scope.Name = "get_plan_offers"
	for _, raw := range []string{`{"user_id":8}`, `{"channel_id":7}`, `{"sql":"select 1"}`, `{"amount":1}`} {
		in.Call.Arguments = json.RawMessage(raw)
		if _, err := service.dispatch(context.Background(), nil, in, access.Principal{}, [32]byte{}); !errors.Is(err, access.ErrInvalid) {
			t.Fatal(err)
		}
	}
	if p.calls != 0 {
		t.Fatal("model authority reached read")
	}
}

type unavailableOffersCatalogue struct{ channels offersCatalogue }

func (c unavailableOffersCatalogue) CheckoutCatalogue(context.Context) planoffers.CheckoutObservation {
	return planoffers.CheckoutObservation{Channels: c.channels}
}
func TestPlanOffersUnverifiedPaymentAndFixedProductAmountAreUnavailable(t *testing.T) {
	f := offersFixture()
	all := offersCatalogue{{ChannelID: 7, Binding: strings.Repeat("a", 64), Subscription: true}, {ChannelID: 7, Binding: strings.Repeat("b", 64)}}
	p := &planOffersPort{facts: f}
	reader, _ := NewCorePlanOffers(p, &planOffersAuthority{}, unavailableOffersCatalogue{channels: all})
	raw, err := reader.PlanOffers(context.Background(), "synthetic-original-session")
	if err != nil {
		t.Fatal(err)
	}
	var v map[string]any
	if json.Unmarshal(raw, &v) != nil || v["checkout_available"] != false || v["payment_rpc_verified"] != false || len(v["topup_discounts"].([]any)) != 0 || len(v["plans"].([]any)) != 1 {
		t.Fatal(string(raw), err)
	}
	if v["topup_channels"].([]any)[0].(map[string]any)["reason"] != "payment_service_not_verified" {
		t.Fatal(v)
	}
	f.Plans = []planoffers.Plan{}
	for _, fixed := range []int64{0, 1, 10000, 10001, -1} {
		all[1].FixedAmountMinor = fixed
		v = readOffers(t, f, all[1:])
		want := fixed >= 0 && fixed <= 10000
		if v["checkout_available"] != want {
			t.Fatal(fixed, v)
		}
	}
}

func TestPlanOffersCarryCoreCreditMinimumAndExcludeIneligibleDiscounts(t *testing.T) {
	facts := offersFixture()
	quote := facts.TopupChannels[0].CreditQuote
	if quote.MinimumCreditUnits() != 1 || !facts.Valid(7) {
		t.Fatal("older Core quote did not retain positive-quantity default")
	}
	for _, invalid := range []int64{0, -1} {
		quote.MinCreditUnits = offersInt(invalid)
		if facts.Valid(7) {
			t.Fatal("invalid minimum accepted", invalid)
		}
	}
	quote.MinCreditUnits = offersInt(9007199254740993)
	if !facts.Valid(7) {
		t.Fatal("exact positive int64 minimum rejected")
	}
	catalogue := offersCatalogue{{ChannelID: 7, Binding: strings.Repeat("b", 64)}}
	value := readOffers(t, facts, catalogue)
	channel := value["topup_channels"].([]any)[0].(map[string]any)
	discount := value["topup_discounts"].([]any)[0].(map[string]any)
	if channel["min_credit_units"] != "9007199254740993" ||
		discount["min_credit_units"] != "9007199254740993" ||
		len(discount["discounts"].(map[string]any)) != 0 {
		t.Fatal(channel, discount)
	}
}
