package payments

import (
	"context"
	"errors"
	waffoorder "github.com/waffo-com/waffo-go/v2/types/order"
	"net/http"
	"testing"
)

func TestWaffoPendingLookupBindsOriginalPayment(t *testing.T) {
	w, _ := refundWaffoFixture(t)
	order, _ := refundWaffoOrder(w)
	order.Refunds = nil
	for _, field := range []string{"valid", "orderAmount", "orderCurrency", "acquiringOrderId"} {
		t.Run(field, func(t *testing.T) {
			data := map[string]any{"paymentRequestId": order.ID, "merchantOrderId": order.ID, "acquiringOrderId": order.PaymentID, "orderStatus": "PAY_IN_PROGRESS", "orderAmount": decimalMinor(order.Intent.AmountMinor), "orderCurrency": "USD", "merchantInfo": map[string]string{"merchantId": w.Channel().Merchant}}
			switch field {
			case "orderAmount":
				data[field] = "0.01"
			case "orderCurrency":
				data[field] = "CNY"
			case "acquiringOrderId":
				data[field] = "different_transaction"
			}
			w.client = &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
				if r.URL.Path != "/api/v1/order/inquiry" {
					t.Fatalf("unexpected request: %s", r.URL.Path)
				}
				return response(refundWaffoJSON(t, map[string]any{"code": "0", "data": data})), nil
			})}
			events, err := w.Lookup(context.Background(), order)
			if field == "valid" {
				if err != nil || len(events) != 0 {
					t.Fatalf("events=%v err=%v", events, err)
				}
			} else if !errors.Is(err, ErrConflict) {
				t.Fatalf("mismatched %s accepted: %v", field, err)
			}
		})
	}
}

// v2.3.0 InquiryOrderData marks acquiringOrderId omitempty. Do not infer a
// mandatory pending transaction from a successful-payment fixture.
func TestWaffoPendingLookupAllowsUnassignedAcquiringOrder(t *testing.T) {
	w, _ := refundWaffoFixture(t)
	order, _ := refundWaffoOrder(w)
	order.Refunds = nil
	for _, saved := range []string{"", "WAF_fixture"} {
		for _, state := range []string{"PAY_IN_PROGRESS", "PAY_SUCCESS"} {
			t.Run(saved+"/"+state, func(t *testing.T) {
				order.PaymentID = saved
				data := waffoorder.InquiryOrderData{PaymentRequestID: order.ID, MerchantOrderID: order.ID, OrderStatus: state, OrderCurrency: "USD", OrderAmount: decimalMinor(order.Intent.AmountMinor), MerchantInfo: &waffoorder.MerchantInfo{MerchantID: w.Channel().Merchant}}
				w.client = &http.Client{Transport: transportFunc(func(*http.Request) (*http.Response, error) {
					return response(refundWaffoJSON(t, map[string]any{"code": "0", "data": data})), nil
				})}
				events, err := w.Lookup(context.Background(), order)
				if state == "PAY_SUCCESS" {
					if !errors.Is(err, ErrConflict) {
						t.Fatalf("paid without transaction: %v", err)
					}
				} else if err != nil || len(events) != 0 {
					t.Fatalf("pending acquiringOrderId omitted: events=%v err=%v", events, err)
				}
			})
		}
	}
}
