// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"context"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	waffoutils "github.com/waffo-com/waffo-go/v2/utils"
)

func refundWaffoFixture(t *testing.T) (*Waffo, string) {
	t.Helper()
	private, public := gatewayKey(t)
	der, err := x509.MarshalPKIXPublicKey(public)
	if err != nil {
		t.Fatal(err)
	}
	w, err := NewWaffo(WaffoConfig{Channel: Channel{ID: 3, Provider: "waffo", Merchant: "merchant_fixture", Environment: "test", Currency: "USD", Enabled: true}, APIKey: "waffo_fixture_api_key", PrivateKey: private, NotifyURL: "https://example.com/callbacks/3", ReturnURL: "https://example.com/paid"})
	if err != nil {
		t.Fatal(err)
	}
	return w, base64.StdEncoding.EncodeToString(der)
}
func refundWaffoData(t *testing.T, r Refund, status string) map[string]any {
	t.Helper()
	binding, err := json.Marshal(waffoRefundBinding{Hold: r.HoldReceipt, Refund: r.ID})
	if err != nil {
		t.Fatal(err)
	}
	return map[string]any{"refundRequestId": stableKey("submit", r.ID), "merchantRefundOrderId": r.ID, "acquiringOrderId": "WAF_fixture", "acquiringRefundOrderId": "WAF_refund_fixture", "origPaymentRequestId": "top_1", "refundAmount": decimalMinor(r.AmountMinor), "refundStatus": status, "userCurrency": "USD", "extendInfo": string(binding)}
}
func refundWaffoJSON(t *testing.T, value any) string {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}
func refundWaffoOrder(w *Waffo) (Order, Refund) {
	order := gatewayOrder(w.Channel())
	order.PaymentID = "WAF_fixture"
	refund := Refund{ID: stableKey("refund", order.ID, "synthetic_request"), AmountMinor: 300, HoldReceipt: "hold_synthetic_receipt", State: "held", Dispatch: DispatchUnknown}
	order.Refunds[refund.ID] = refund
	return order, refund
}

func TestWaffoRefundDispatchUsesFrozenHoldAndSignedOfficialRequest(t *testing.T) {
	w, public := refundWaffoFixture(t)
	order, refund := refundWaffoOrder(w)
	queries, writes := 0, 0
	w.client = &http.Client{Transport: transportFunc(func(request *http.Request) (*http.Response, error) {
		raw, err := io.ReadAll(request.Body)
		if err != nil || !waffoutils.Verify(string(raw), request.Header.Get("X-Signature"), public) {
			t.Fatal("unsigned refund request")
		}
		if request.URL.Host != "api-sandbox.waffo.com" || request.Method != "POST" {
			t.Fatal("unexpected endpoint", request.URL)
		}
		if request.URL.Path == "/api/v1/order/inquiry" {
			queries++
			var envelope struct {
				Result json.RawMessage `json:"result"`
			}
			if json.Unmarshal([]byte(waffoPaid), &envelope) != nil {
				t.Fatal("fixture")
			}
			return response(`{"code":"0","data":` + string(envelope.Result) + `}`), nil
		}
		if request.URL.Path != "/api/v1/order/refund" {
			t.Fatal("unexpected mutation", request.URL)
		}
		writes++
		var fields map[string]string
		if jsonObject(raw, &fields) != nil || fields["refundRequestId"] != stableKey("submit", refund.ID) || fields["merchantRefundOrderId"] != refund.ID || fields["acquiringOrderId"] != order.PaymentID || fields["refundAmount"] != "3.00" || fields["merchantId"] != w.Channel().Merchant || fields["refundNotifyUrl"] != w.cfg.NotifyURL {
			t.Fatal("changed immutable refund snapshot", fields)
		}
		var binding waffoRefundBinding
		if jsonObject([]byte(fields["extendInfo"]), &binding) != nil || binding.Hold != refund.HoldReceipt || binding.Refund != refund.ID {
			t.Fatal("missing Core hold binding")
		}
		return response(refundWaffoJSON(t, map[string]any{"code": "0", "data": refundWaffoData(t, refund, "REFUND_IN_PROGRESS")})), nil
	})}
	result, err := w.Refund(context.Background(), order, refund, stableKey("submit", refund.ID))
	if err != nil || result.ID != "WAF_refund_fixture" || result.Status != "REFUND_IN_PROGRESS" || queries != 1 || writes != 1 {
		t.Fatalf("dispatch result=%+v queries=%d writes=%d err=%v", result, queries, writes, err)
	}
	if !w.RefundEnabled() || !w.PartialRefunds() || w.ReplayWindow("refund_submit") != 0 {
		t.Fatal("incorrect refund capability/replay policy")
	}
	invalid := order
	invalid.Intent.Merchant = "different_merchant"
	_, err = w.Refund(context.Background(), invalid, refund, stableKey("submit", refund.ID))
	var before refundBeforeDispatch
	if !errors.As(err, &before) || writes != 1 {
		t.Fatal("unbound refund reached provider")
	}
}

func TestWaffoRefundCallbacksPreserveEvidenceAndFailCoreProofGate(t *testing.T) {
	w, _ := refundWaffoFixture(t)
	order, refund := refundWaffoOrder(w)
	for _, status := range []string{"REFUND_IN_PROGRESS", "ORDER_PARTIALLY_REFUNDED", "ORDER_FULLY_REFUNDED", "ORDER_REFUND_FAILED"} {
		raw := refundWaffoJSON(t, map[string]any{"eventType": "REFUND_NOTIFICATION", "result": refundWaffoData(t, refund, status)})
		event, err := w.Verify(gatewayCallback(raw, "X-Signature"), time.Now())
		if err != nil || event.AmountMinor != 300 || event.RefundID != "WAF_refund_fixture" || event.LocalRefundID != refund.ID || event.Evidence.EventID != stableKey(event.RefundID, "REFUND_NOTIFICATION", status) || string(event.Evidence.Payload) != raw || event.Evidence.AcceptedProofID != "" {
			t.Fatalf("refund evidence was lost or became proof: %+v %v", event, err)
		}
		memory, core := newMemory(), newCore(w.Channel())
		service, err := New(memory, callbackRejectProofCore{Core: core}, w)
		if err != nil {
			t.Fatal(err)
		}
		if err := memory.Insert(context.Background(), order); err != nil {
			t.Fatal(err)
		}
		if err := service.Receive(context.Background(), w.Channel().ID, gatewayCallback(raw, "X-Signature")); !errors.Is(err, ErrEvidence) || len(memory.events) != 0 || core.count("release") != 0 || core.count("commit") != 0 {
			t.Fatal("refund parser bypassed independent Core evidence", err)
		}
		if _, err := w.Verify(gatewayCallback(strings.Replace(raw, `"USD"`, `"CNY"`, 1), "X-Signature"), time.Now()); err == nil {
			t.Fatal("changed refund currency accepted")
		}
	}
}

func TestWaffoUnknownRefundLookupUsesOriginalRequestWithoutResubmitting(t *testing.T) {
	w, _ := refundWaffoFixture(t)
	order, refund := refundWaffoOrder(w)
	reads, writes := 0, 0
	w.client = &http.Client{Transport: transportFunc(func(request *http.Request) (*http.Response, error) {
		if request.URL.Path == "/api/v1/order/inquiry" {
			var envelope struct {
				Result json.RawMessage `json:"result"`
			}
			_ = json.Unmarshal([]byte(waffoPaid), &envelope)
			return response(`{"code":"0","data":` + string(envelope.Result) + `}`), nil
		}
		if request.URL.Path != "/api/v1/refund/inquiry" {
			writes++
			return nil, errors.New("unexpected mutation")
		}
		reads++
		var fields map[string]string
		if json.NewDecoder(request.Body).Decode(&fields) != nil || fields["refundRequestId"] != stableKey("submit", refund.ID) {
			t.Fatal("UNKNOWN lookup changed request identity")
		}
		return response(refundWaffoJSON(t, map[string]any{"code": "0", "data": refundWaffoData(t, refund, "ORDER_PARTIALLY_REFUNDED")})), nil
	})}
	events, err := w.Lookup(context.Background(), order)
	if err != nil || reads != 1 || writes != 0 || len(events) != 2 {
		t.Fatalf("lookup failed: events=%+v reads=%d writes=%d err=%v", events, reads, writes, err)
	}
	found := events[1]
	if found.Type != RefundSucceeded || found.LocalRefundID != refund.ID || found.Evidence.Source != "api" || len(found.Evidence.Signature) != 0 || found.Evidence.AcceptedProofID != "" {
		t.Fatal("API observation manufactured trusted proof", found)
	}
}
