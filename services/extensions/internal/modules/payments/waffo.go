// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"encoding/json"
	"net/http"
	"strconv"
	"time"

	waffoutils "github.com/waffo-com/waffo-go/v2/utils"
)

// Waffo Global and Waffo Pancake are different protocols and credentials.
// This adapter follows the Global v2.1.0 SDK's order API and RSA request signing.
type WaffoConfig struct {
	Channel       Channel
	APIKey        string
	PrivateKey    string
	NotifyURL     string
	ReturnURL     string
	PayMethodType string
	PayMethodName string
}

type Waffo struct {
	cfg    WaffoConfig
	client *http.Client
}

func NewWaffo(cfg WaffoConfig) (*Waffo, error) {
	if !cfg.Channel.valid() || cfg.Channel.Provider != "waffo" || cfg.Channel.Currency != "USD" || !validProviderCredential(cfg.APIKey) || waffoutils.ValidatePrivateKey(cfg.PrivateKey) != nil {
		return nil, ErrInvalid
	}
	for _, u := range []string{cfg.NotifyURL, cfg.ReturnURL} {
		if _, err := secureURL(u); err != nil {
			return nil, err
		}
	}
	return &Waffo{cfg: cfg, client: outboundClient()}, nil
}

func (w *Waffo) Channel() Channel                { return w.cfg.Channel }
func (*Waffo) RefundEnabled() bool               { return true }
func (*Waffo) PartialRefunds() bool              { return true }
func (*Waffo) ReplayWindow(string) time.Duration { return 0 }

func (w *Waffo) api(ctx context.Context, path string, value any) ([]byte, error) {
	body, err := json.Marshal(value)
	if err != nil {
		return nil, ErrInvalid
	}
	signature, err := waffoutils.Sign(string(body), w.cfg.PrivateKey)
	if err != nil {
		return nil, ErrInvalid
	}
	origin := "https://api.waffo.com"
	if w.cfg.Channel.Environment == "test" {
		origin = "https://api-sandbox.waffo.com"
	}
	raw, err := requestJSONBytes(ctx, w.client, http.MethodPost, origin+"/api/v1/"+path, body,
		http.Header{"X-Api-Key": {w.cfg.APIKey}, "X-Signature": {signature}, "X-Api-Version": {"1.0.0"}})
	if err != nil {
		return nil, err
	}
	var envelope struct {
		Code string          `json:"code"`
		Data json.RawMessage `json:"data"`
	}
	if jsonObject(raw, &envelope) != nil || envelope.Code != "0" || len(envelope.Data) == 0 {
		return nil, ErrUnavailable
	}
	return envelope.Data, nil
}

func (w *Waffo) Checkout(ctx context.Context, o Order, _ string) (Checkout, error) {
	if !providerOrderMatches(o, w.cfg.Channel) {
		return Checkout{}, ErrInvalid
	}
	raw, err := w.api(ctx, "order/create", map[string]any{
		"paymentRequestId": o.ID, "merchantOrderId": o.ID,
		"orderAmount": decimalMinor(o.Intent.AmountMinor), "orderCurrency": o.Intent.Currency,
		"orderDescription": "LMM API account top-up", "orderRequestedAt": time.Now().UTC().Format("2006-01-02T15:04:05.000Z"),
		"notifyUrl":          w.cfg.NotifyURL,
		"merchantInfo":       map[string]string{"merchantId": w.cfg.Channel.Merchant},
		"userInfo":           map[string]string{"userId": strconv.FormatInt(o.Intent.CreatedBy, 10), "userTerminal": "WEB"},
		"paymentInfo":        map[string]string{"productName": "ONE_TIME_PAYMENT", "payMethodType": w.cfg.PayMethodType, "payMethodName": w.cfg.PayMethodName},
		"goodsInfo":          map[string]string{"goodsName": "Account top-up", "appName": "LMM API"},
		"successRedirectUrl": w.cfg.ReturnURL, "failedRedirectUrl": w.cfg.ReturnURL,
	})
	if err != nil {
		return Checkout{}, err
	}
	var v waffoPayment
	if jsonObject(raw, &v) != nil || v.PaymentRequestID != o.ID || v.MerchantOrderID != o.ID || !validID(v.AcquiringOrderID) {
		return Checkout{}, ErrConflict
	}
	checkoutURL := v.OrderAction
	var action struct {
		WebURL string `json:"webUrl"`
	}
	if json.Unmarshal([]byte(v.OrderAction), &action) == nil {
		checkoutURL = action.WebURL
	}
	if _, err = secureURL(checkoutURL); err != nil {
		return Checkout{}, ErrConflict
	}
	return Checkout{ID: v.AcquiringOrderID, URL: checkoutURL, PaymentID: v.AcquiringOrderID}, nil
}

type waffoPayment struct {
	PaymentRequestID string `json:"paymentRequestId"`
	MerchantOrderID  string `json:"merchantOrderId"`
	AcquiringOrderID string `json:"acquiringOrderId"`
	OrderStatus      string `json:"orderStatus"`
	OrderAction      string `json:"orderAction"`
	OrderCurrency    string `json:"orderCurrency"`
	OrderAmount      string `json:"orderAmount"`
	MerchantInfo     struct {
		MerchantID string `json:"merchantId"`
	} `json:"merchantInfo"`
}

func (w *Waffo) payment(v waffoPayment) (Event, error) {
	if v.OrderStatus != "PAY_SUCCESS" {
		return Event{}, ErrUnsupported
	}
	amount, err := minorDecimal(v.OrderAmount)
	if err != nil || v.OrderCurrency != w.cfg.Channel.Currency || v.MerchantInfo.MerchantID != w.cfg.Channel.Merchant ||
		!validTopupOrderID(v.MerchantOrderID) || v.PaymentRequestID != v.MerchantOrderID || !validID(v.AcquiringOrderID) {
		return Event{}, ErrConflict
	}
	c := w.cfg.Channel
	return Event{OrderID: v.MerchantOrderID, Type: Paid, Currency: c.Currency, AmountMinor: amount,
		Evidence: Evidence{Provider: c.Provider, Merchant: c.Merchant, Environment: c.Environment, Transaction: v.AcquiringOrderID,
			EventID: stableKey(v.AcquiringOrderID, "PAYMENT_NOTIFICATION", v.OrderStatus)}}, nil
}

func (w *Waffo) Verify(r *http.Request, _ time.Time) (Event, error) {
	raw, signature, err := callbackBytes(r, "X-Signature")
	if err != nil {
		return Event{}, err
	}
	var v struct {
		Type   string          `json:"eventType"`
		Result json.RawMessage `json:"result"`
	}
	if jsonObject(raw, &v) != nil {
		return Event{}, ErrInvalid
	}
	var e Event
	switch v.Type {
	case "PAYMENT_NOTIFICATION":
		var payment waffoPayment
		if jsonObject(v.Result, &payment) != nil {
			return Event{}, ErrInvalid
		}
		e, err = w.payment(payment)
	case "REFUND_NOTIFICATION":
		var refund waffoRefund
		if jsonObject(v.Result, &refund) != nil {
			return Event{}, ErrInvalid
		}
		e, err = w.refundEvent(refund)
	default:
		return Event{}, ErrUnsupported
	}
	if err != nil {
		return Event{}, err
	}
	e.Evidence.Payload, e.Evidence.Signature, e.Evidence.Source = raw, signature, "webhook"
	return e, nil
}

func (w *Waffo) Lookup(ctx context.Context, o Order) ([]Event, error) {
	if !providerOrderMatches(o, w.cfg.Channel) {
		return nil, ErrInvalid
	}
	raw, err := w.api(ctx, "order/inquiry", map[string]string{"paymentRequestId": o.ID})
	if err != nil {
		return nil, err
	}
	var v waffoPayment
	if jsonObject(raw, &v) != nil || v.MerchantOrderID != o.ID || v.PaymentRequestID != o.ID || v.MerchantInfo.MerchantID != w.cfg.Channel.Merchant {
		return nil, ErrConflict
	}
	var events []Event
	if v.OrderStatus == "PAY_SUCCESS" {
		e, err := w.payment(v)
		if err != nil || e.AmountMinor != o.Intent.AmountMinor || o.PaymentID != "" && e.Evidence.Transaction != o.PaymentID {
			return nil, ErrConflict
		}
		e.Evidence.Payload = raw
		apiEvidence(&e, e.Evidence.Transaction, Paid)
		events = append(events, e)
	}
	refunds, err := w.lookupRefunds(ctx, o)
	if err != nil {
		return nil, err
	}
	return append(events, refunds...), nil
}

// Global requires an RSA-signed acknowledgement, unlike the other gateways.
func (w *Waffo) webhookAcknowledgement() ([]byte, http.Header, error) {
	body := []byte(`{"message":"success"}`)
	signature, err := waffoutils.Sign(string(body), w.cfg.PrivateKey)
	if err != nil {
		return nil, nil, ErrUnavailable
	}
	return body, http.Header{"Content-Type": {"application/json"}, "X-Signature": {signature}}, nil
}
