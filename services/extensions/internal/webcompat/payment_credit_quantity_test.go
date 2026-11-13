package webcompat

import (
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"strings"
	"testing"
)

type quantityCheckoutPort struct {
	checkoutPort
	observed payments.PrepareRequest
}

func (p *quantityCheckoutPort) Preview(_ context.Context, _ string, r payments.PrepareRequest) (payments.Intent, error) {
	p.previews++
	p.observed = r
	return payments.Intent{ID: 21, AccountID: 42, CreatedBy: 17, Currency: r.Currency, AmountMinor: 1327, CreditUnits: 1000000, CreditUnit: "credit_500k_usd", Key: r.Key}, nil
}
func (p *quantityCheckoutPort) Create(_ context.Context, _ string, r payments.PrepareRequest) (payments.Order, error) {
	p.creates++
	p.observed = r
	return payments.Order{ID: "top_21", Intent: payments.Intent{Currency: r.Currency, AmountMinor: 1327, CreditUnits: 1000000, CreditUnit: "credit_500k_usd"}}, nil
}
func TestBrowserCreditQuantityUsesCoreCharge(t *testing.T) {
	p := &quantityCheckoutPort{}
	mux := checkoutMux(t, p)
	body := `{"channel_id":"5003","amount_minor":"0","requested_credit_units":"1000000","currency":"CNY","request_key":"quantity_browser_0001"}`
	w := checkoutCall(mux, "POST", "/api/user/payments/quote", webToken, body)
	if w.Code != 200 || p.observed.RequestedCreditUnits != 1000000 || p.observed.AmountMinor != 0 || p.creates != 0 || !strings.Contains(w.Body.String(), `"amount_minor":"1327"`) {
		t.Fatal(w.Code, w.Body.String(), p)
	}
	confirm := strings.TrimSuffix(body, "}") + `,"order_id":"top_21","expected_credit_units":"1000000"}`
	w = checkoutCall(mux, "POST", "/api/user/payments/checkout", webToken, confirm)
	if w.Code != 202 || p.creates != 1 || p.observed.AccountID != 42 || p.observed.RequestedCreditUnits != 1000000 || p.observed.AmountMinor != 0 {
		t.Fatal(w.Code, w.Body.String(), p)
	}
	w = checkoutCall(mux, "POST", "/api/user/payments/checkout", webToken, strings.Replace(confirm, `"expected_credit_units":"1000000"`, `"expected_credit_units":"999999"`, 1))
	if w.Code != 409 || p.creates != 1 {
		t.Fatal("unreviewed credit entered checkout", w.Code)
	}
	for _, qty := range []string{`"0"`, `"-1"`, `"01"`, `1000000`, `"9223372036854775808"`} {
		w = checkoutCall(mux, "POST", "/api/user/payments/quote", webToken, strings.Replace(body, `"1000000"`, qty, 1))
		if w.Code != 400 {
			t.Fatal("invalid quantity accepted", qty, w.Code)
		}
	}
}

func TestBrowserDiscountCanonicalCodeAndReviewedAmount(t *testing.T) {
	p := &quantityCheckoutPort{}
	mux := checkoutMux(t, p)
	body := `{"channel_id":"5003","amount_minor":"0","requested_credit_units":"1000000","currency":"CNY","request_key":"discount_browser_0001","discount_code":" save20 "}`
	w := checkoutCall(mux, "POST", "/api/user/payments/quote", webToken, body)
	if w.Code != 200 || p.observed.DiscountCode != "SAVE20" || !strings.Contains(w.Body.String(), `"discount_code":"SAVE20"`) {
		t.Fatal(w.Code, w.Body.String(), p.observed)
	}
	confirm := strings.TrimSuffix(body, "}") + `,"order_id":"top_21","expected_credit_units":"1000000","expected_amount_minor":"1327"}`
	w = checkoutCall(mux, "POST", "/api/user/payments/checkout", webToken, confirm)
	if w.Code != 202 || p.observed.DiscountCode != "SAVE20" || p.observed.AmountMinor != 0 || p.creates != 1 {
		t.Fatal(w.Code, w.Body.String(), p.observed)
	}
	for _, amount := range []string{"0", "1328", ""} {
		wrong := strings.Replace(confirm, `"expected_amount_minor":"1327"`, `"expected_amount_minor":"`+amount+`"`, 1)
		w = checkoutCall(mux, "POST", "/api/user/payments/checkout", webToken, wrong)
		if w.Code != 409 || p.creates != 1 {
			t.Fatal("unreviewed amount dispatched", amount, w.Code)
		}
	}
	empty := strings.Replace(body, " save20 ", "   ", 1)
	w = checkoutCall(mux, "POST", "/api/user/payments/quote", webToken, empty)
	if w.Code != 200 || strings.Contains(w.Body.String(), "discount_code") || p.observed.DiscountCode != "" {
		t.Fatal(w.Code, w.Body.String())
	}
}
