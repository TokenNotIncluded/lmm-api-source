// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments_test

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments/rustbridge"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/webcompat"
	_ "github.com/lib/pq"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

func TestDiscountCheckoutRealRust(t *testing.T) {
	path := os.Getenv("DISCOUNT_JOINT_FIXTURE")
	if path == "" {
		t.Skip("owned real Core fixture required")
	}
	var f struct {
		Socket        string `json:"socket"`
		IdentityToken string `json:"identity_token"`
		PaymentToken  string `json:"payment_token"`
		Owner         string `json:"owner"`
		URLFile       string `json:"module_url_file"`
		DB            string `json:"module_db"`
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(raw, &f); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Second)
	defer cancel()
	raw, err = os.ReadFile(f.URLFile)
	if err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if err != nil {
		t.Fatal("module open")
	}
	defer db.Close()
	store, err := p.NewPostgres(ctx, db, f.DB)
	if err != nil {
		t.Fatal("module guard", err)
	}
	identity, err := coreclient.New(f.Socket, []byte(f.IdentityToken))
	if err != nil {
		t.Fatal(err)
	}
	defer identity.Close()
	payment, err := coreclient.New(f.Socket, []byte(f.PaymentToken))
	if err != nil {
		t.Fatal(err)
	}
	defer payment.Close()
	bridge, err := rustbridge.New(payment)
	if err != nil {
		t.Fatal(err)
	}
	raw, err = os.ReadFile(os.Getenv("DISCOUNT_PANCAKE_KEY_FILE"))
	if err != nil {
		t.Fatal(err)
	}
	adapter, err := p.NewPancake(p.PancakeConfig{Channel: p.Channel{ID: 7, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "CNY", Enabled: true, CreditQuantity: true}, PrivateKey: string(raw), StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", TaxCategory: "digital_goods"})
	if err != nil {
		t.Fatal(err)
	}
	service, err := p.New(store, bridge, adapter)
	if err != nil {
		t.Fatal(err)
	}
	auth, err := webcompat.NewAuthenticator(identity, []byte("independent_host_service_credential_000001"))
	if err != nil {
		t.Fatal(err)
	}
	mux := http.NewServeMux()
	webcompat.RegisterPaymentCheckout(mux, auth, service, nil)
	server := httptest.NewServer(mux)
	defer server.Close()
	client := &http.Client{Timeout: 10 * time.Second}
	call := func(route string, input map[string]string, want int) map[string]string {
		t.Helper()
		raw, _ := json.Marshal(input)
		req, _ := http.NewRequestWithContext(ctx, "POST", server.URL+"/api/user/payments/"+route, bytes.NewReader(raw))
		req.Header.Set("Authorization", "Bearer "+f.Owner)
		req.Header.Set("Content-Type", "application/json")
		resp, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		body, _ := io.ReadAll(resp.Body)
		if resp.StatusCode != want {
			t.Fatalf("%s HTTP %d want %d: %s", route, resp.StatusCode, want, body)
		}
		if want >= 400 {
			return nil
		}
		var v struct {
			Success bool                       `json:"success"`
			Data    map[string]json.RawMessage `json:"data"`
		}
		if json.Unmarshal(body, &v) != nil || !v.Success {
			t.Fatal("bad envelope")
		}
		out := map[string]string{}
		for k, b := range v.Data {
			var s string
			if json.Unmarshal(b, &s) == nil {
				out[k] = s
			}
		}
		return out
	}
	input := map[string]string{"channel_id": "7", "currency": "CNY", "amount_minor": "0", "requested_credit_units": "1000000", "discount_code": " save20 ", "request_key": "independent_discount_checkout_01"}
	// Invalid and expired input must fail safely before the sole valid intent exists.
	for _, code := range []string{"MISSING20", "EXPIRED20"} {
		input["discount_code"] = code
		input["request_key"] = "independent_invalid_" + code
		call("quote", input, 400)
	}
	input["discount_code"] = " save20 "
	input["request_key"] = "independent_discount_checkout_01"
	quote := call("quote", input, 200)
	if quote["discount_code"] != "SAVE20" || quote["amount_minor"] != "1109" || quote["credit_units"] != "1000000" {
		t.Fatal("canonical code or frozen quantity/amount incorrect", quote)
	}
	input["discount_code"] = "OTHER20"
	call("quote", input, 409)
	input["discount_code"] = "SAVE20"
	input["order_id"] = quote["order_id"]
	input["expected_credit_units"] = "1000000"
	input["expected_amount_minor"] = "1110"
	call("checkout", input, 409)
	input["expected_amount_minor"] = "1109"
	first := call("checkout", input, 202)
	retry := call("checkout", input, 202)
	if first["order_id"] != quote["order_id"] || first["state"] != "checkout_pending" || first["checkout_url"] != retry["checkout_url"] || first["order_id"] != retry["order_id"] || first["amount_minor"] != "1109" || first["credit_units"] != "1000000" {
		t.Fatal("checkout/retry diverged")
	}
	var count int
	if db.QueryRowContext(ctx, "SELECT count(*) FROM payment_orders").Scan(&count) != nil || count != 1 {
		t.Fatal("duplicate durable order")
	}
	o, err := store.Get(ctx, quote["order_id"])
	if err != nil {
		t.Fatal(err)
	}
	if len(o.Jobs) != 1 {
		t.Fatal("duplicate checkout job")
	}
	for _, j := range o.Jobs {
		if j.Kind != "checkout" {
			t.Fatal("unexpected job")
		}
	}
	t.Log("real loopback HTTP -> Rust Tonic UDS -> PG: canonical discount, exact quantity/amount, 400 safe edits, 409 conflicts and deterministic retry; no provider I/O")
}
