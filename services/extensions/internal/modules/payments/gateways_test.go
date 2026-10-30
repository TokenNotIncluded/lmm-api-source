package payments

import (
	"bytes"
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	waffoutils "github.com/waffo-com/waffo-go/v2/utils"
)

func gatewayKey(t *testing.T) (string, *rsa.PublicKey) {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der})), &key.PublicKey
}

func gatewayOrder(c Channel) Order {
	return Order{ID: "top_1", ChannelID: c.ID, Namespace: c.namespace(),
		Intent: Intent{ID: 1, AccountID: 99, CreatedBy: 7, Provider: c.Provider, Merchant: c.Merchant, Environment: c.Environment,
			Currency: c.Currency, AmountMinor: 1000, CreditUnits: 5000000, CreditUnit: "credit_500k_usd", Key: "gateway_create_0001", RequestHash: make([]byte, 32)},
		CreditState: "pending", Refunds: map[string]Refund{}, Jobs: map[string]Job{}}
}

func creemFixture(t *testing.T) *Creem {
	t.Helper()
	c, err := NewCreem(CreemConfig{Channel: Channel{ID: 2, Provider: "creem", Merchant: "merchant_fixture", Environment: "test", Currency: "USD", Enabled: true},
		APIKey: "creem_test_fixture_key", ProductID: "prod_fixture", ProductAmountMinor: 1000, SuccessURL: "https://example.com/paid"})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

const creemPaid = `{"id":"evt_creem","eventType":"checkout.completed","object":{"id":"ch_fixture","object":"checkout","mode":"test","request_id":"top_1","status":"completed","product":{"id":"prod_fixture","billing_type":"onetime"},"metadata":{"lmm_merchant":"merchant_fixture"},"order":{"transaction":"tran_fixture","status":"paid","amount_paid":1000,"currency":"USD","mode":"test"}}}`
const pancakePaid = `{"id":"evt_pancake","eventId":"PAY_fixture","eventType":"order.completed","mode":"test","storeId":"STO_0123456789abcdefghijkl","data":{"orderId":"ORD_fixture","orderMerchantExternalId":"top_1","orderStatus":"completed","paymentId":"PAY_fixture","paymentStatus":"succeeded","currency":"USD","amount":"10.00","chargedAmount":"10.00","orderMetadata":{"lmm_product_id":"PROD_0123456789abcdefghijkl","lmm_merchant":"MER_0123456789abcdefghijkl"}}}`
const waffoPaid = `{"eventType":"PAYMENT_NOTIFICATION","result":{"paymentRequestId":"top_1","merchantOrderId":"top_1","acquiringOrderId":"WAF_fixture","orderStatus":"PAY_SUCCESS","orderCurrency":"USD","orderAmount":"10.00","merchantInfo":{"merchantId":"merchant_fixture"}}}`

func gatewayCallback(raw, header string) *http.Request {
	r := httptest.NewRequest(http.MethodPost, "https://example.com/callbacks/2", strings.NewReader(raw))
	r.Header.Set(header, "original-provider-signature")
	return r
}

func TestCreemFixedProductCheckoutAndObservedLookup(t *testing.T) {
	c := creemFixture(t)
	unsupportedCurrency := c.cfg
	unsupportedCurrency.Channel.Currency = "JPY"
	if _, err := NewCreem(unsupportedCurrency); !errors.Is(err, ErrInvalid) {
		t.Fatal("zero-decimal currency entered the two-decimal wallet profile", err)
	}
	o := gatewayOrder(c.Channel())
	calls := 0
	c.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.URL.Host != "test-api.creem.io" || r.Header.Get("X-Api-Key") != c.cfg.APIKey {
			t.Fatal("wrong configured API", r.URL)
		}
		if r.URL.Path == "/v1/products" {
			if r.Method != http.MethodGet || r.URL.Query().Get("product_id") != "prod_fixture" {
				t.Fatal("wrong product validation request")
			}
			return response(`{"id":"prod_fixture","object":"product","mode":"test","price":1000,"currency":"USD","billing_type":"onetime","status":"active","tax_mode":"inclusive"}`), nil
		}
		if r.URL.Path != "/v1/checkouts" {
			t.Fatal("unexpected provider endpoint", r.URL.Path)
		}
		if r.Method == http.MethodGet {
			if r.URL.Query().Get("checkout_id") != "ch_fixture" {
				t.Fatal("lookup changed checkout identity")
			}
			var envelope struct {
				Object json.RawMessage `json:"object"`
			}
			_ = json.Unmarshal([]byte(creemPaid), &envelope)
			return response(string(envelope.Object)), nil
		}
		var request map[string]any
		if json.NewDecoder(r.Body).Decode(&request) != nil || request["product_id"] != "prod_fixture" || request["request_id"] != o.ID || request["units"] != float64(1) {
			t.Fatal("checkout changed product/order/units", request)
		}
		if _, ok := request["amount"]; ok {
			t.Fatal("invented arbitrary Creem price override")
		}
		return response(`{"id":"ch_fixture","object":"checkout","mode":"test","request_id":"top_1","product":"prod_fixture","checkout_url":"https://checkout.creem.io/ch_fixture"}`), nil
	})}
	bad := o
	bad.Intent.AmountMinor++
	if _, err := c.Checkout(context.Background(), bad, "key"); !errors.Is(err, ErrInvalid) || calls != 0 {
		t.Fatal("wrong amount reached provider", err, calls)
	}
	checkout, err := c.Checkout(context.Background(), o, "key")
	if err != nil || checkout.ID != "ch_fixture" {
		t.Fatal(checkout, err)
	}
	o.Checkout = checkout
	events, err := c.Lookup(context.Background(), o)
	if err != nil || len(events) != 1 || events[0].Evidence.Source != "api" || len(events[0].Evidence.Signature) != 0 || events[0].Evidence.AcceptedProofID != "" {
		t.Fatal("API observation was lost or became proof", events, err)
	}
	core := newCore(c.Channel())
	s, err := New(newMemory(), core, c)
	if err != nil {
		t.Fatal(err)
	}
	_, err = s.Create(context.Background(), "user", PrepareRequest{Key: "wrong_product_price_001", AccountID: 99, ChannelID: c.Channel().ID, Currency: "USD", AmountMinor: 999})
	if !errors.Is(err, ErrInvalid) || core.count("prepare") != 0 {
		t.Fatal("invalid product price created a Core intent", err)
	}
}

func TestCreemRefusesChangedPriceAndAdditionalTaxBeforeCheckout(t *testing.T) {
	for _, change := range []string{`"price":1100`, `"tax_mode":"exclusive"`, `"billing_type":"recurring"`} {
		c := creemFixture(t)
		writes := 0
		product := `{"id":"prod_fixture","object":"product","mode":"test","price":1000,"currency":"USD","billing_type":"onetime","status":"active","tax_mode":"inclusive"}`
		switch {
		case strings.Contains(change, "price"):
			product = strings.Replace(product, `"price":1000`, change, 1)
		case strings.Contains(change, "tax_mode"):
			product = strings.Replace(product, `"tax_mode":"inclusive"`, change, 1)
		default:
			product = strings.Replace(product, `"billing_type":"onetime"`, change, 1)
		}
		c.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
			if r.Method == http.MethodPost {
				writes++
			}
			return response(product), nil
		})}
		if _, err := c.Checkout(context.Background(), gatewayOrder(c.Channel()), "key"); !errors.Is(err, ErrConflict) || writes != 0 {
			t.Fatal("unquoted amount reached checkout", change, err, writes)
		}
	}
}

func TestGatewayCallbacksPreserveEvidenceAndRequireCore(t *testing.T) {
	private, _ := gatewayKey(t)
	w, err := NewWaffo(WaffoConfig{Channel: Channel{ID: 3, Provider: "waffo", Merchant: "merchant_fixture", Environment: "test", Currency: "USD", Enabled: true},
		APIKey: "waffo_fixture_api_key", PrivateKey: private, NotifyURL: "https://example.com/callbacks/3", ReturnURL: "https://example.com/paid"})
	if err != nil {
		t.Fatal(err)
	}
	p := &Pancake{cfg: PancakeConfig{Channel: Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD", Enabled: true}, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl"}}
	for _, tc := range []struct {
		name, raw, header string
		adapter           Adapter
	}{
		{"creem", creemPaid, "Creem-Signature", creemFixture(t)},
		{"waffo", waffoPaid, "X-Signature", w},
		{"pancake", pancakePaid, "X-Waffo-Signature", p},
	} {
		t.Run(tc.name, func(t *testing.T) {
			event, err := tc.adapter.Verify(gatewayCallback(tc.raw, tc.header), time.Now())
			if err != nil || string(event.Evidence.Payload) != tc.raw || string(event.Evidence.Signature) != "original-provider-signature" || event.Evidence.AcceptedProofID != "" || event.AmountMinor != 1000 {
				t.Fatal("raw evidence changed", event, err)
			}
			memory := newMemory()
			core := newCore(tc.adapter.Channel())
			s, err := New(memory, callbackRejectProofCore{Core: core}, tc.adapter)
			if err != nil {
				t.Fatal(err)
			}
			if err = memory.Insert(context.Background(), gatewayOrder(tc.adapter.Channel())); err != nil {
				t.Fatal(err)
			}
			err = s.Receive(context.Background(), tc.adapter.Channel().ID, gatewayCallback(tc.raw, tc.header))
			if !errors.Is(err, ErrEvidence) || len(memory.events) != 0 || core.count("credit") != 0 {
				t.Fatal("unverified callback bypassed Core", err)
			}
			for _, raw := range []string{strings.Replace(tc.raw, `"USD"`, `"CNY"`, 1), strings.Replace(tc.raw, `"test"`, `"prod"`, 1), strings.Replace(tc.raw, `"eventType":`, `"eventType":"forged","eventType":`, 1)} {
				if raw == tc.raw {
					continue
				}
				if _, err = tc.adapter.Verify(gatewayCallback(raw, tc.header), time.Now()); err == nil {
					t.Fatal("conflicting currency/environment/duplicate JSON accepted")
				}
			}
		})
	}
	for _, raw := range []string{
		strings.Replace(pancakePaid, `"chargedAmount":"10.00",`, "", 1),
		strings.Replace(pancakePaid, `"chargedAmount":"10.00"`, `"chargedAmount":"0.00"`, 1),
		strings.Replace(pancakePaid, `"chargedAmount":"10.00"`, `"chargedAmount":"9.00"`, 1),
	} {
		if !strings.Contains(raw, "chargedAmount") {
			raw = strings.Replace(raw, `"amount":"10.00"`, `"amount":"10.00","listPrice":{"total":"10.00","taxAmount":"0.00"}`, 1)
		}
		if _, err := p.Verify(gatewayCallback(raw, "X-Waffo-Signature"), time.Now()); err == nil {
			t.Fatal("Pancake snapshot/missing/contradictory charged amount accepted")
		}
	}
}

func TestWaffoRequestAndAcknowledgementUseOfficialRSASigning(t *testing.T) {
	private, public := gatewayKey(t)
	pubDER, err := x509.MarshalPKIXPublicKey(public)
	if err != nil {
		t.Fatal(err)
	}
	w, err := NewWaffo(WaffoConfig{Channel: Channel{ID: 3, Provider: "waffo", Merchant: "merchant_fixture", Environment: "test", Currency: "USD", Enabled: true},
		APIKey: "waffo_fixture_api_key", PrivateKey: private, NotifyURL: "https://example.com/callbacks/3", ReturnURL: "https://example.com/paid"})
	if err != nil {
		t.Fatal(err)
	}
	w.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		body, err := io.ReadAll(r.Body)
		if err != nil || !waffoutils.Verify(string(body), r.Header.Get("X-Signature"), base64.StdEncoding.EncodeToString(pubDER)) {
			t.Fatal("request signature differs from official SDK")
		}
		var fields map[string]any
		_ = json.Unmarshal(body, &fields)
		if r.URL.Host != "api-sandbox.waffo.com" || r.Header.Get("X-Api-Version") != "1.0.0" || fields["paymentRequestId"] != "top_1" || fields["merchantOrderId"] != "top_1" || fields["orderAmount"] != "10.00" || fields["orderCurrency"] != "USD" {
			t.Fatal("wrong immutable Global checkout", fields)
		}
		return response(`{"code":"0","data":{"paymentRequestId":"top_1","merchantOrderId":"top_1","acquiringOrderId":"WAF_fixture","orderAction":"{\"actionType\":\"REDIRECT\",\"webUrl\":\"https://checkout.waffo.com/pay\"}"}}`), nil
	})}
	checkout, err := w.Checkout(context.Background(), gatewayOrder(w.Channel()), "gateway_stable_key")
	if err != nil || checkout.PaymentID != "WAF_fixture" {
		t.Fatal(checkout, err)
	}
	body, headers, err := w.webhookAcknowledgement()
	if err != nil || string(body) != `{"message":"success"}` || !waffoutils.Verify(string(body), headers.Get("X-Signature"), base64.StdEncoding.EncodeToString(pubDER)) {
		t.Fatal("Global acknowledgement lacks valid merchant signature", err)
	}
}

func TestPancakeSDKCheckoutBindsCoreIntentAndStableKey(t *testing.T) {
	private, public := gatewayKey(t)
	var checkouts, tokens atomic.Int64
	httpClient := &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatal(err)
		}
		digest := sha256.Sum256(body)
		canonical := "POST\n" + r.URL.Path + "\n" + r.Header.Get("X-Timestamp") + "\n" + base64.StdEncoding.EncodeToString(digest[:])
		hash := sha256.Sum256([]byte(canonical))
		sig, err := base64.StdEncoding.DecodeString(r.Header.Get("X-Signature"))
		if err != nil || rsa.VerifyPKCS1v15(public, crypto.SHA256, hash[:], sig) != nil || r.Header.Get("X-Merchant-Id") != "MER_0123456789abcdefghijkl" || r.URL.Host != "api.waffo.ai" {
			t.Fatal("official Pancake request signature/merchant differs")
		}
		var fields map[string]any
		_ = json.Unmarshal(body, &fields)
		switch r.URL.Path {
		case "/v1/actions/auth/issue-session-token":
			tokens.Add(1)
			if fields["buyerIdentity"] != "lmm-user-7" || fields["productId"] != "PROD_0123456789abcdefghijkl" || r.Header.Get("X-Idempotency-Key") != "" {
				t.Fatal("bad customer session binding", fields)
			}
			return response(`{"data":{"token":"synthetic_token","expiresAt":"2026-10-30T01:00:00Z"}}`), nil
		case "/v1/actions/checkout/create-session":
			checkouts.Add(1)
			price, _ := fields["priceSnapshot"].(map[string]any)
			metadata, _ := fields["metadata"].(map[string]any)
			billing, _ := fields["billingDetail"].(map[string]any)
			if fields["orderMerchantExternalId"] != "top_1" || price["amount"] != "10.00" || fields["currency"] != "USD" || metadata["lmm_product_id"] != "PROD_0123456789abcdefghijkl" || metadata["lmm_core_intent_id"] != "1" || billing["country"] != "CN" || r.Header.Get("X-Idempotency-Key") != "gateway_checkout_key_001" {
				t.Fatal("Pancake checkout changed frozen price or order", fields)
			}
			return response(`{"data":{"sessionId":"CHK_fixture","checkoutUrl":"https://pancake.waffo.ai/store/test/checkout/CHK_fixture","expiresAt":"2026-10-30T01:00:00Z"}}`), nil
		default:
			t.Fatal("unexpected SDK request", r.URL.Path)
			return nil, ErrInvalid
		}
	})}
	p, err := newPancake(PancakeConfig{Channel: Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD", Enabled: true},
		PrivateKey: private, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", ReturnURL: "https://example.com/paid", CheckoutRegion: "china", CheckoutLanguage: "zh-Hans", TaxCategory: "saas"}, httpClient)
	if err != nil {
		t.Fatal(err)
	}
	checkout, err := p.Checkout(context.Background(), gatewayOrder(p.Channel()), "gateway_checkout_key_001")
	if err != nil || checkout.ID != "CHK_fixture" || checkouts.Load() != 1 || tokens.Load() != 1 || !strings.HasSuffix(checkout.URL, "#token=synthetic_token") {
		t.Fatal(checkout, err, checkouts.Load(), tokens.Load())
	}
}

type epaySignerFunc func(context.Context, int64) (Checkout, error)

func (f epaySignerFunc) EPayCheckout(ctx context.Context, id int64) (Checkout, error) {
	return f(ctx, id)
}

func TestCoreEPayKeepsMD5KeyOutOfGoAndPreservesLDC(t *testing.T) {
	cfg := EPayConfig{Channel: Channel{ID: 5, Provider: "epay", Merchant: "123", Environment: "live", Currency: "LDC", Enabled: true}, PaymentType: "epay"}
	calls := 0
	method := http.MethodPost
	signer := epaySignerFunc(func(_ context.Context, id int64) (Checkout, error) {
		calls++
		if id != 1 {
			t.Fatal("signer received another intent", id)
		}
		return Checkout{ID: "top_1", URL: "https://credit.linux.do/epay/pay/submit.php?out_trade_no=top_1&sign=fixture", Method: method}, nil
	})
	e, err := NewCoreEPay(cfg, signer)
	if err != nil {
		t.Fatal(err)
	}
	checkout, err := e.Checkout(context.Background(), gatewayOrder(e.Channel()), "key")
	if err != nil || calls != 1 || checkout.Method != http.MethodPost {
		t.Fatal("LDC checkout lost POST method", checkout, err, calls)
	}
	method = http.MethodGet
	if _, err = e.Checkout(context.Background(), gatewayOrder(e.Channel()), "key"); !errors.Is(err, ErrConflict) {
		t.Fatal("LDC accepted the wrong checkout method", err)
	}
	cfg.MerchantKey = "must_not_be_in_go"
	if _, err = NewCoreEPay(cfg, signer); !errors.Is(err, ErrInvalid) {
		t.Fatal("production Go accepted MD5 key", err)
	}
	values := url.Values{"pid": {"123"}, "type": {"epay"}, "out_trade_no": {"top_1"}, "trade_no": {"ldc_fixture"}, "trade_status": {"TRADE_SUCCESS"}, "money": {"10.00"}, "sign": {strings.Repeat("a", 32)}}
	req := httptest.NewRequest(http.MethodGet, "https://example.com/callbacks/5?"+values.Encode(), nil)
	event, err := e.Verify(req, time.Now())
	if err != nil || event.Currency != "LDC" || string(event.Evidence.Payload) != values.Encode() || event.Evidence.AcceptedProofID != "" {
		t.Fatal("LDC evidence mislabelled or self-approved", event, err)
	}
	for _, signType := range []string{"", "SHA256", "MD5"} {
		values.Set("sign_type", signType)
		_, err = e.Verify(httptest.NewRequest(http.MethodGet, "https://example.com/callbacks/5?"+values.Encode(), nil), time.Now())
		if (signType == "MD5") != (err == nil) {
			t.Fatal("LDC sign_type contract", signType, err)
		}
	}
	cfg.MerchantKey = ""
	cfg.PaymentType = "linuxdo"
	if _, err = NewCoreEPay(cfg, signer); !errors.Is(err, ErrInvalid) {
		t.Fatal("LDC accepted an unsupported payment type", err)
	}
	cfg.PaymentType = "epay"
	cfg.Channel.Currency = "CNY"
	cny, err := NewCoreEPay(cfg, signer)
	if err != nil {
		t.Fatal(err)
	}
	values.Del("sign_type")
	if _, err = cny.Verify(httptest.NewRequest(http.MethodGet, "https://example.com/callbacks/5?"+values.Encode(), nil), time.Now()); !errors.Is(err, ErrDenied) {
		t.Fatal("CNY signature type requirement changed", err)
	}
	for _, retired := range []string{"fastpay", "FastPay"} {
		cfg.PaymentType = retired
		if _, err = NewCoreEPay(cfg, signer); !errors.Is(err, ErrInvalid) {
			t.Fatal("retired FastPay method entered the EPay catalog", err)
		}
	}
	if _, err = e.Lookup(context.Background(), gatewayOrder(e.Channel())); !errors.Is(err, ErrUnsupported) {
		t.Fatal("EPay lookup attempted with unavailable Core secret", err)
	}
}

type personalGatewayCore struct{ *fakeCore }

func (p personalGatewayCore) PreparePersonal(ctx context.Context, user string, req PrepareRequest) (Intent, error) {
	req.AccountID = 99
	return p.fakeCore.Prepare(ctx, user, req)
}

func TestGatewayCatalogAndPersonalPreviewDoNotQueueCheckout(t *testing.T) {
	c := creemFixture(t)
	second := *c
	second.cfg.Channel.ID = 8
	second.cfg.ProductID = "prod_second"
	second.cfg.ProductAmountMinor = 2000
	disabled := second
	disabled.cfg.Channel.ID = 9
	disabled.cfg.Channel.Enabled = false
	disabled.cfg.ProductID = "prod_disabled"
	memory := newMemory()
	core := newCore(c.Channel())
	s, err := New(memory, personalGatewayCore{core}, &second, &disabled, c)
	if err != nil {
		t.Fatal(err)
	}
	catalog := s.Catalog()
	if len(catalog) != 2 || catalog[0].ID != 2 || catalog[1].ID != 8 || catalog[0].FixedAmountMinor != 1000 || catalog[1].ProductID != "prod_second" {
		t.Fatal(catalog)
	}
	encoded, _ := json.Marshal(catalog)
	if bytes.Contains(encoded, []byte(c.cfg.APIKey)) {
		t.Fatal("catalog exposed provider credential")
	}
	duplicate := *c
	duplicate.cfg.Channel.ID = 12
	if _, err := New(memory, core, c, &duplicate); !errors.Is(err, ErrInvalid) {
		t.Fatal("duplicate product channel accepted", err)
	}
	req := PrepareRequest{Key: "personal_gateway_preview_001", ChannelID: 2, Currency: "USD", AmountMinor: 1000}
	intent, err := s.Preview(context.Background(), "user", req)
	if err != nil || intent.AccountID != 99 || intent.AmountMinor != 1000 || len(memory.orders) != 0 {
		t.Fatal("preview queued payment or changed Core intent", intent, err)
	}
	core.deny = true
	if _, err = s.Preview(context.Background(), "user", req); !errors.Is(err, ErrDenied) {
		t.Fatal("preview reused revoked permission", err)
	}
	req.AccountID = 44
	if _, err = s.Preview(context.Background(), "user", req); !errors.Is(err, ErrInvalid) {
		t.Fatal("personal quote accepted caller-selected payer", err)
	}
}

func TestGatewayLookupNeverInventsPancakePaymentAmount(t *testing.T) {
	private, _ := gatewayKey(t)
	client := &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		if r.URL.Path != "/v1/graphql" {
			t.Fatal(r.URL.Path)
		}
		return response(`{"data":{"payments":[{"id":"PAY_fixture","status":"succeeded","orderMerchantExternalId":"top_1"}]}}`), nil
	})}
	p, err := newPancake(PancakeConfig{Channel: Channel{ID: 4, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD", Enabled: true}, PrivateKey: private, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", ReturnURL: "https://example.com/paid", TaxCategory: "saas"}, client)
	if err != nil {
		t.Fatal(err)
	}
	if events, err := p.Lookup(context.Background(), gatewayOrder(p.Channel())); !errors.Is(err, ErrEvidence) || len(events) != 0 {
		t.Fatal("unsigned lookup invented charged amount", events, err)
	}
}
