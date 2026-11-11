package payments

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func dualPancakeAdapters(t *testing.T, client *http.Client) (*Pancake, *Pancake) {
	t.Helper()
	private, _ := gatewayKey(t)
	makeAdapter := func(id int64, currency string) *Pancake {
		a, err := newPancake(PancakeConfig{Channel: Channel{ID: id, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "live", Currency: currency, Enabled: true, CreditQuantity: true}, PrivateKey: private, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", TaxCategory: "saas"}, client)
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	return makeAdapter(5001, "USD"), makeAdapter(5003, "CNY")
}
func TestPancakeDualCurrencyStartupAndCanonicalCheckout(t *testing.T) {
	charges := map[string]string{}
	client := &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		if r.URL.Path == "/v1/actions/auth/issue-session-token" {
			return response(`{"data":{"token":"synthetic","expiresAt":"2026-10-30T01:00:00Z"}}`), nil
		}
		if r.URL.Path != "/v1/actions/checkout/create-session" {
			t.Fatal("unexpected SDK call", r.URL.Path)
		}
		raw, _ := io.ReadAll(r.Body)
		var v struct {
			Currency string `json:"currency"`
			Price    struct {
				Amount string `json:"amount"`
			} `json:"priceSnapshot"`
		}
		if json.Unmarshal(raw, &v) != nil {
			t.Fatal("invalid SDK body")
		}
		charges[v.Currency] = v.Price.Amount
		return response(`{"data":{"sessionId":"CHK_fixture","checkoutUrl":"https://pancake.waffo.ai/store/test/checkout/CHK_fixture","expiresAt":"2026-10-30T01:00:00Z"}}`), nil
	})}
	usd, cny := dualPancakeAdapters(t, client)
	store := newMemory()
	core := &quantityQuoteCore{fakeCore: newCore(usd.Channel())}
	service, err := New(store, core, usd, cny)
	if err != nil {
		t.Fatal("same merchant/product dual currency cannot start", err)
	}
	catalog := service.Catalog()
	if len(catalog) != 2 || catalog[0].ID != 5001 || catalog[1].ID != 5003 || catalog[0].StoreID != catalog[1].StoreID || catalog[0].ProductID != catalog[1].ProductID {
		t.Fatal("dual catalog lost binding", catalog)
	}
	for _, tc := range []struct {
		id, amount int64
		currency   string
	}{{5001, 198, "USD"}, {5003, 1327, "CNY"}} {
		core.amount = tc.amount
		order, err := service.Create(context.Background(), "user", PrepareRequest{Key: "dual_quantity_" + tc.currency + "_01", AccountID: 99, ChannelID: tc.id, Currency: tc.currency, RequestedCreditUnits: 1000000})
		if err != nil {
			t.Fatal(err)
		}
		if order.Namespace != usd.Channel().namespace() || order.Namespace != cny.Channel().namespace() {
			t.Fatal("currency split provider transaction identity")
		}
		if err = service.Step(context.Background(), order.ID); err != nil {
			t.Fatal(err)
		}
	}
	if charges["USD"] != "1.98" || charges["CNY"] != "13.27" {
		t.Fatal("SDK changed Core charges", charges)
	}
	// Configuration dedup includes currency even for adapters without product scoping.
	a := &fakeAdapter{c: usd.Channel()}
	b := &fakeAdapter{c: cny.Channel()}
	if _, err = New(newMemory(), core, a, b); err != nil {
		t.Fatal("unscoped dual currency rejected", err)
	}
	duplicate := *a
	duplicate.c.ID = 5004
	if _, err = New(newMemory(), core, a, &duplicate); err == nil {
		t.Fatal("same-currency merchant duplicate accepted")
	}
}

// Payload currency selects a route only. This controlled Core refuses proof
// admission, so no syntactically valid callback can assert financial authority.
type dualProofRejectCore struct {
	*quantityQuoteCore
	seen    []int64
	payload []byte
}

func (c *dualProofRejectCore) AcceptProof(_ context.Context, id int64, e Evidence) (ProofReceipt, error) {
	c.seen = append(c.seen, id)
	c.payload = append([]byte(nil), e.Payload...)
	return ProofReceipt{}, ErrEvidence
}
func TestPancakeDualCurrencyNativeCallbackRequiresCoreProof(t *testing.T) {
	client := &http.Client{Transport: transportFunc(func(*http.Request) (*http.Response, error) {
		t.Fatal("callback performed SDK I/O")
		return nil, ErrInvalid
	})}
	usd, cny := dualPancakeAdapters(t, client)
	core := &dualProofRejectCore{quantityQuoteCore: &quantityQuoteCore{fakeCore: newCore(usd.Channel()), amount: 1327}}
	service, err := New(newMemory(), core, usd, cny)
	if err != nil {
		t.Fatal(err)
	}
	order, err := service.Create(context.Background(), "user", PrepareRequest{Key: "dual_native_cny_0001", AccountID: 99, ChannelID: 5003, Currency: "CNY", RequestedCreditUnits: 1000000})
	if err != nil {
		t.Fatal(err)
	}
	raw := fmt.Sprintf(`{"id":"evt_native_cny","eventType":"order.completed","mode":"prod","storeId":"STO_0123456789abcdefghijkl","data":{"orderMerchantExternalId":%q,"orderId":"ORD_native_cny","paymentId":"PAY_native_cny","currency":"CNY","amount":"13.27","chargedAmount":"13.27","orderMetadata":{"lmm_product_id":"PROD_0123456789abcdefghijkl","lmm_merchant":"MER_0123456789abcdefghijkl"}}}`, order.ID)
	handler := service.WebhookHandler()
	for _, channel := range []string{"5001", "5003"} {
		request := httptest.NewRequest("POST", "/callbacks/"+channel, strings.NewReader(raw))
		request.Header.Set("X-Waffo-Signature", "synthetic-untrusted-signature")
		result := httptest.NewRecorder()
		handler.ServeHTTP(result, request)
		if result.Code == 200 {
			t.Fatal("unverified callback acknowledged", channel)
		}
		if channel == "5001" && len(core.seen) != 0 {
			t.Fatal("CNY callback sent to USD proof authority")
		}
	}
	if len(core.seen) != 1 || core.seen[0] != order.Intent.ID || string(core.payload) != raw || core.count("credit") != 0 {
		t.Fatal("callback lost canonical binding or bypassed Core", core.seen)
	}
}
