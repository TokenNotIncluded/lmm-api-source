// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"net/http"
	"reflect"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

func TestWalletPresetOwnerSaveAndWalletReadback(t *testing.T) {
	path := t.TempDir()
	port := &checkoutPort{catalog: []payments.PublicChannel{
		{Channel: payments.Channel{ID: 1, Currency: "CNY", CreditQuantity: true}, MinCreditUnits: 500000},
		{Channel: payments.Channel{ID: 2, Currency: "USD", MinAmountMinor: 100}},
		{Channel: payments.Channel{ID: 3, Currency: "USD"}, FixedAmountMinor: 400},
		{Channel: payments.Channel{ID: 4, Currency: "USD", Provider: "coinbase"}},
		{Channel: payments.Channel{ID: 5, Currency: "USD", Provider: "coinbase", CreditQuantity: true}, MinCreditUnits: 500000},
	}}
	mount := func(level int32) http.Handler {
		core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
			self := sessionSelf()
			self.PlatformLevel = level
			return self, nil
		}}
		host, _ := mountTransportHost(t, nil)
		if err := host.MarkReady(); err != nil {
			t.Fatal(err)
		}
		return Mount(host, newAuth(t, core), nil, nil, AdditionalServices{
			Plugins:         &pluginControllerFake{enabled: map[string]bool{"payments": false}},
			PaymentCheckout: port, WalletPresetDirectory: path,
		})
	}
	admin := mount(6)
	const endpoint = "/api/user/payments/presets"
	put := func(mux http.Handler, currency, body string, want int) {
		t.Helper()
		r := checkoutCall(mux, "PUT", endpoint+"/"+currency, webToken, body)
		if r.Code != want {
			t.Fatalf("save %s status %d: %s", currency, r.Code, r.Body.String())
		}
	}
	put(admin, "CNY", `{"amounts_minor":["667","7000"]}`, 200)
	put(admin, "USD", `{"amounts_minor":[]}`, 200)
	put(admin, "USDC", `{"amounts_minor":["350","850"]}`, 200)
	put(admin, "CNY", `{"amounts_minor":["0"]}`, 400)
	put(mount(5), "CNY", `{"amounts_minor":["999"]}`, 403)

	reloaded := mount(6)
	adminView := checkoutCall(reloaded, "GET", endpoint, webToken, "")
	var config struct{ Data map[string][]string }
	if adminView.Code != 200 || json.Unmarshal(adminView.Body.Bytes(), &config) != nil ||
		!reflect.DeepEqual(config.Data, map[string][]string{"CNY": {"667", "7000"}, "USD": {}, "USDC": {"350", "850"}}) {
		t.Fatalf("read-back: %d %s", adminView.Code, adminView.Body.String())
	}
	wallet := checkoutCall(mount(1), "GET", "/api/user/payments/channels", webToken, "")
	var catalog struct {
		Data struct {
			Channels []struct {
				Amounts *[]string `json:"preset_amounts_minor"`
				Minimum string    `json:"min_amount_minor"`
			}
		}
	}
	if wallet.Code != http.StatusOK || json.Unmarshal(wallet.Body.Bytes(), &catalog) != nil || len(catalog.Data.Channels) != 5 {
		t.Fatalf("wallet response: %d %s", wallet.Code, wallet.Body.String())
	}
	channels := catalog.Data.Channels
	if channels[0].Amounts == nil || !reflect.DeepEqual(*channels[0].Amounts, config.Data["CNY"]) ||
		channels[1].Amounts == nil || len(*channels[1].Amounts) != 0 || channels[1].Minimum != "100" ||
		channels[2].Amounts != nil || channels[3].Amounts == nil || !reflect.DeepEqual(*channels[3].Amounts, config.Data["USDC"]) || channels[4].Amounts == nil || len(*channels[4].Amounts) != 0 || port.previews != 0 || port.creates != 0 {
		t.Fatalf("wallet options: %s", wallet.Body.String())
	}
}
