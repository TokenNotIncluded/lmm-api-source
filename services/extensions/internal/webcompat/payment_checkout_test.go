package webcompat

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

type checkoutPort struct {
	creates    int
	previews   int
	credential string
	request    payments.PrepareRequest
	deny       bool
	checkout   payments.Checkout
}

func (*checkoutPort) Catalog() []payments.PublicChannel {
	return []payments.PublicChannel{{Channel: payments.Channel{ID: 3, Provider: "creem", Merchant: "private_configured_merchant", Environment: "test", Currency: "USD", Enabled: true}, FixedAmountMinor: 100, ProductID: "prod_test"}}
}
func (p *checkoutPort) Preview(_ context.Context, user string, r payments.PrepareRequest) (payments.Intent, error) {
	p.previews++
	p.credential = user
	if r.AccountID != 0 {
		return payments.Intent{}, payments.ErrDenied
	}
	return payments.Intent{ID: 21, AccountID: 42, CreatedBy: 17, Provider: "creem", Environment: "test", Currency: r.Currency, AmountMinor: r.AmountMinor, CreditUnits: 9007199254740993, CreditUnit: "credit_500k_usd", Key: r.Key}, nil
}
func (p *checkoutPort) Create(_ context.Context, user string, r payments.PrepareRequest) (payments.Order, error) {
	p.creates++
	p.credential = user
	p.request = r
	return payments.Order{ID: "top_21", Intent: payments.Intent{AccountID: r.AccountID, Currency: r.Currency, AmountMinor: r.AmountMinor, CreditUnits: 9007199254740993, CreditUnit: "credit_500k_usd"}}, nil
}
func (p *checkoutPort) Get(_ context.Context, user, id string) (payments.Order, error) {
	p.credential = user
	if p.deny {
		return payments.Order{}, payments.ErrDenied
	}
	return payments.Order{ID: id, Intent: payments.Intent{Currency: "USD", AmountMinor: 100, CreditUnits: 500000, CreditUnit: "credit_500k_usd"}, Checkout: p.checkout}, nil
}
func checkoutMux(t *testing.T, p PaymentCheckoutService) *http.ServeMux {
	t.Helper()
	a, err := NewAuthenticator(&authCore{}, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	m := http.NewServeMux()
	RegisterPaymentCheckout(m, a, p)
	return m
}
func checkoutCall(m *http.ServeMux, method, path, token, body string) *httptest.ResponseRecorder {
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	if token != "" {
		r.Header.Set("Authorization", "Bearer "+token)
	}
	r.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	m.ServeHTTP(w, r)
	return w
}

func TestBrowserPaymentRequiresReviewedCoreQuote(t *testing.T) {
	p := &checkoutPort{}
	m := checkoutMux(t, p)
	body := `{"channel_id":"3","amount_minor":"100","currency":"USD","request_key":"payment_request_000001"}`
	w := checkoutCall(m, "POST", "/api/user/payments/quote", webToken, body)
	if w.Code != 200 || p.creates != 0 || p.credential != webToken {
		t.Fatal(w.Code, w.Body.String(), p)
	}
	var response struct {
		Data paymentQuoteView `json:"data"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if response.Data.CreditUnits != "9007199254740993" || response.Data.AccountID != "42" {
		t.Fatal("lost exact Core quote", response.Data)
	}
	changed := strings.TrimSuffix(body, "}") + `,"order_id":"top_21","expected_credit_units":"500000"}`
	w = checkoutCall(m, "POST", "/api/user/payments/checkout", webToken, changed)
	if w.Code != 409 || p.creates != 0 {
		t.Fatal("unreviewed quote dispatched", w.Code)
	}
	confirmed := strings.Replace(changed, `"expected_credit_units":"500000"`, `"expected_credit_units":"9007199254740993"`, 1)
	w = checkoutCall(m, "POST", "/api/user/payments/checkout", webToken, confirmed)
	if w.Code != 202 || p.creates != 1 || p.request.AccountID != 42 || p.request.Key != "payment_request_000001" {
		t.Fatal(w.Code, w.Body.String(), p)
	}
	if !strings.Contains(w.Body.String(), `"core_credit_confirmed":false`) || !strings.Contains(w.Body.String(), `"amount_minor":"100"`) {
		t.Fatal(w.Body.String())
	}
	if !strings.Contains(w.Body.String(), `"checkout_method":"GET"`) {
		t.Fatal("hosted checkout omitted its opening method", w.Body.String())
	}
}

func TestBrowserPaymentRejectsCallerPayerAndDuplicateFields(t *testing.T) {
	p := &checkoutPort{}
	m := checkoutMux(t, p)
	base := `{"channel_id":"3","amount_minor":"100","currency":"USD","request_key":"payment_request_000001"`
	for _, body := range []string{base + `,"account_id":"17"}`, base + `,"amount_minor":"200"}`, strings.Replace(base, "\"100\"", "100", 1) + "}", strings.Replace(base, "\"3\"", "\"03\"", 1) + "}"} {
		w := checkoutCall(m, "POST", "/api/user/payments/quote", webToken, body)
		if w.Code != 400 || p.previews != 0 || p.creates != 0 {
			t.Fatal("invalid body entered Core", w.Code, body)
		}
	}
	for _, token := range []string{"", hostToken} {
		w := checkoutCall(m, "POST", "/api/user/payments/quote", token, base+"}")
		if w.Code != 401 || p.previews != 0 {
			t.Fatal("non-session entered checkout", w.Code)
		}
	}
}

func TestBrowserPaymentCatalogAndReadsAreSeparateFromMoney(t *testing.T) {
	p := &checkoutPort{}
	m := checkoutMux(t, p)
	w := checkoutCall(m, "GET", "/api/user/payments/channels", webToken, "")
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"fixed_amount_minor":"100"`) || strings.Contains(w.Body.String(), "private_configured_merchant") || p.creates != 0 {
		t.Fatal(w.Code, w.Body.String())
	}
	p.checkout = payments.Checkout{ID: "top_21", Method: "POST", URL: "https://credit.linux.do/epay/pay/submit.php?out_trade_no=top_21&sign=fixture"}
	w = checkoutCall(m, "GET", "/api/user/payments/checkout/top_21", webToken, "")
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"checkout_method":"POST"`) {
		t.Fatal("Core form checkout changed method", w.Code, w.Body.String())
	}
	p.deny = true
	w = checkoutCall(m, "GET", "/api/user/payments/checkout/top_21", webToken, "")
	if w.Code != 403 || p.credential != webToken {
		t.Fatal("cross-user order exposed", w.Code)
	}
	w = checkoutCall(checkoutMux(t, nil), "GET", "/api/user/payments/channels", webToken, "")
	if w.Code != 503 {
		t.Fatal("missing payment host advertised available", w.Code)
	}
}
