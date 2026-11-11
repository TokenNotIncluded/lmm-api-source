package payments

import (
	"context"
	"crypto"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"testing"
)

func TestPancakeSDKUsesCanonicalCNYQuantityCharge(t *testing.T) {
	private, public := gatewayKey(t)
	checkouts := 0
	client := &http.Client{Transport: transportFunc(func(r *http.Request) (*http.Response, error) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatal(err)
		}
		digest := sha256.Sum256(body)
		canonical := "POST\n" + r.URL.Path + "\n" + r.Header.Get("X-Timestamp") + "\n" + base64.StdEncoding.EncodeToString(digest[:])
		hash := sha256.Sum256([]byte(canonical))
		sig, err := base64.StdEncoding.DecodeString(r.Header.Get("X-Signature"))
		if err != nil || rsa.VerifyPKCS1v15(public, crypto.SHA256, hash[:], sig) != nil {
			t.Fatal("SDK signature changed")
		}
		switch r.URL.Path {
		case "/v1/actions/auth/issue-session-token":
			return response(`{"data":{"token":"synthetic_token","expiresAt":"2026-10-30T01:00:00Z"}}`), nil
		case "/v1/actions/checkout/create-session":
			checkouts++
			var fields struct {
				Currency string `json:"currency"`
				Price    struct {
					Amount string `json:"amount"`
				} `json:"priceSnapshot"`
			}
			if json.Unmarshal(body, &fields) != nil || fields.Currency != "CNY" || fields.Price.Amount != "13.27" || r.Header.Get("X-Idempotency-Key") != "quantity_sdk_fixture_01" {
				t.Fatal("SDK replaced Core's CNY amount", string(body))
			}
			return response(`{"data":{"sessionId":"CHK_fixture","checkoutUrl":"https://pancake.waffo.ai/store/test/checkout/CHK_fixture","expiresAt":"2026-10-30T01:00:00Z"}}`), nil
		default:
			t.Fatal("unexpected SDK call")
			return nil, ErrInvalid
		}
	})}
	adapter, err := newPancake(PancakeConfig{Channel: Channel{ID: 3, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "CNY", Enabled: true, CreditQuantity: true}, PrivateKey: private, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", TaxCategory: "saas"}, client)
	if err != nil {
		t.Fatal(err)
	}
	order := gatewayOrder(adapter.Channel())
	order.Intent.AmountMinor = 1327
	order.Intent.CreditUnits = 1000000
	order.Intent.RequestedCreditUnits = 1000000
	if _, err = adapter.Checkout(context.Background(), order, "quantity_sdk_fixture_01"); err != nil || checkouts != 1 {
		t.Fatal(err, checkouts)
	}
}
