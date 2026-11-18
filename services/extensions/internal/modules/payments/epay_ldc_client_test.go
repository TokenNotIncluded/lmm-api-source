package payments

import (
	"context"
	"errors"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestEPayLDCPreservesOpaqueClientIDForCoreVerification(t *testing.T) {
	merchant := strings.Repeat("a1b2", 16)
	cfg := EPayConfig{Channel: Channel{ID: 5004, Provider: "epay", Merchant: merchant, Environment: "live", Currency: "LDC", MinAmountMinor: 10000}, PaymentType: "epay"}
	signer := epaySignerFunc(func(context.Context, int64) (Checkout, error) {
		t.Fatal("callback parsing dispatched checkout")
		return Checkout{}, ErrUnavailable
	})
	adapter, err := NewCoreEPay(cfg, signer)
	if err != nil || adapter.Channel().Merchant != merchant || adapter.Channel().Enabled || adapter.cfg.MerchantKey != "" {
		t.Fatal("opaque LDC identity was rejected, rewritten or granted a secret", err)
	}
	fields := url.Values{"pid": {merchant}, "type": {"epay"}, "trade_no": {"provider_fixture"}, "out_trade_no": {"top_7"}, "money": {"100.00"}, "trade_status": {"TRADE_SUCCESS"}, "sign": {strings.Repeat("a", 32)}}
	request := httptest.NewRequest("GET", "https://example.test/callbacks/5004?"+fields.Encode(), nil)
	event, err := adapter.Verify(request, time.Now())
	if err != nil || event.Evidence.Merchant != merchant || event.Evidence.AcceptedProofID != "" || string(event.Evidence.Payload) != fields.Encode() {
		t.Fatal("raw opaque pid evidence did not remain bound for Core verification", err)
	}
	fields.Set("pid", strings.Repeat("b3c4", 16))
	request = httptest.NewRequest("GET", "https://example.test/callbacks/5004?"+fields.Encode(), nil)
	if _, err = adapter.Verify(request, time.Now()); !errors.Is(err, ErrDenied) {
		t.Fatal("another opaque client was accepted", err)
	}
	cfg.Channel.Currency = "CNY"
	cfg.PaymentType = "alipay"
	if _, err = NewCoreEPay(cfg, signer); !errors.Is(err, ErrInvalid) {
		t.Fatal("CNY merchant validation was widened", err)
	}
	cfg.Channel.Currency = "LDC"
	cfg.PaymentType = "epay"
	for _, invalid := range []string{"client&other=1", "client space", "client/child", strings.Repeat("a", 129)} {
		cfg.Channel.Merchant = invalid
		if _, err = NewCoreEPay(cfg, signer); !errors.Is(err, ErrInvalid) {
			t.Fatal("unsafe client ID accepted", err)
		}
	}
}
