package payments

import (
	"context"
	"errors"
	"net/http"
	"testing"
)

func TestEPayCashProfileRejectsLinuxDOCreditHost(t *testing.T) {
	for _, endpoint := range []string{"https://credit.linux.do/epay", "https://CREDIT.LINUX.DO./epay"} {
		cfg := epayFixture(t).cfg
		cfg.Endpoint = endpoint
		if _, err := NewEPay(cfg); !errors.Is(err, ErrInvalid) {
			t.Fatalf("cash endpoint %s: %v", endpoint, err)
		}
	}
}

func TestEPayLookupBindsOriginalTransaction(t *testing.T) {
	for _, trade := range []string{"", "different_transaction", "t1"} {
		t.Run(trade, func(t *testing.T) {
			e := epayFixture(t)
			e.client = &http.Client{Transport: transportFunc(func(*http.Request) (*http.Response, error) {
				return response(`{"code":1,"pid":1001,"trade_no":"` + trade + `","out_trade_no":"top_1","money":"10.00","status":1,"type":"alipay"}`), nil
			})}
			events, err := e.Lookup(context.Background(), Order{ID: "top_1", PaymentID: "t1", Intent: Intent{Currency: "CNY", AmountMinor: 1000}})
			if trade == "t1" {
				if err != nil || len(events) != 1 {
					t.Fatalf("events=%v err=%v", events, err)
				}
			} else if !errors.Is(err, ErrConflict) {
				t.Fatalf("unbound transaction accepted: %v", err)
			}
		})
	}
}
