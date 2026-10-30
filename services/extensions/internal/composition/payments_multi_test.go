package composition

import (
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

type coreCheckoutSigner struct{}

func (coreCheckoutSigner) EPayCheckout(context.Context, int64) (payments.Checkout, error) {
	return payments.Checkout{}, payments.ErrUnavailable
}

func TestPaymentV2RestoresLegacyProvidersWithoutFastPay(t *testing.T) {
	key := paymentPrivate(t, "api-key", "synthetic_provider_api_key_000001")
	stripeKey := paymentPrivate(t, "stripe-key", "sk_test_synthetic_provider_key_000001")
	rsaKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalPKCS8PrivateKey(rsaKey)
	if err != nil {
		t.Fatal(err)
	}
	privateKey := paymentPrivate(t, "rsa-key", string(pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der})))
	raw := fmt.Sprintf(`version=2
[[channels]]
id=1
provider='stripe'
merchant='acct_synthetic'
environment='test'
currency='USD'
enabled=true
api_key_file='%s'
api_version='2025-02-24.acacia'
success_url='https://example.test/paid'
cancel_url='https://example.test/cancel'
[[channels]]
id=2
provider='epay'
merchant='1002'
environment='live'
currency='LDC'
enabled=true
endpoint='https://credit.linux.do/epay'
payment_type='epay'
notify_url='https://example.test/callbacks/2'
return_url='https://example.test/wallet'
[[channels]]
id=3
provider='creem'
merchant='creem_synthetic'
environment='test'
currency='USD'
enabled=true
api_key_file='%s'
product_id='prod_synthetic'
product_amount_minor=100
success_url='https://example.test/wallet'
[[channels]]
id=4
provider='waffo'
merchant='waffo_synthetic'
environment='test'
currency='USD'
enabled=true
api_key_file='%s'
private_key_file='%s'
notify_url='https://example.test/callbacks/4'
return_url='https://example.test/wallet'
[[channels]]
id=5
provider='waffo_pancake'
merchant='MER_0123456789abcdefghijkl'
environment='test'
currency='USD'
enabled=true
private_key_file='%s'
store_id='STO_0123456789abcdefghijkl'
product_id='PROD_0123456789abcdefghijkl'
return_url='https://example.test/wallet'
tax_category='saas'
`, stripeKey, key, key, privateKey, privateKey)
	adapters, err := loadMultiPaymentAdapters([]byte(raw), coreCheckoutSigner{})
	if err != nil || len(adapters) != 5 {
		t.Fatal("configured provider rejected", err)
	}
	for i, provider := range []string{"stripe", "epay", "creem", "waffo", "waffo_pancake"} {
		if adapters[i].Channel().Provider != provider {
			t.Fatal(adapters[i].Channel())
		}
	}
	if adapters[1].Channel().Currency != "LDC" {
		t.Fatal("Linux DO mislabelled as fiat")
	}
	for _, bad := range []string{
		strings.Replace(raw, "provider='creem'", "provider='fastpay'", 1),
		strings.ReplaceAll(strings.Replace(raw, "currency='LDC'", "currency='CNY'", 1), "payment_type='epay'", "payment_type='FastPay'"),
		strings.Replace(raw, "version=2", "version=2\nprovider='stripe-test'", 1),
		strings.Replace(raw, "payment_type='epay'", "payment_type='epay'\nmerchant_key_file='/private/epay-key'", 1),
		strings.Replace(raw, "id=2", "id=1", 1),
		strings.Replace(raw, "currency='LDC'", "currency='USD'", 1),
		strings.Replace(raw, "product_amount_minor=100", "product_amount_minor=0", 1),
	} {
		if _, err := loadMultiPaymentAdapters([]byte(bad), coreCheckoutSigner{}); err == nil {
			t.Fatal("unsafe configuration accepted")
		}
	}
	if _, err := loadMultiPaymentAdapters([]byte(raw), nil); err == nil {
		t.Fatal("EPay configured without Core signing")
	}
}
