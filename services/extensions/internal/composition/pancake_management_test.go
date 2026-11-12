package composition

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"encoding/pem"
	"errors"
	"os"
	"reflect"
	"sync"
	"testing"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

// These tests exercise durable configuration and the actual adapter snapshot.
// The quote and order ports are synthetic; no ledger acceptance is claimed.
type configurationOrders struct {
	payments.Store
	orders map[string]payments.Order
}

func (s *configurationOrders) Insert(_ context.Context, order payments.Order) error {
	s.orders[order.ID] = order
	return nil
}
func (s *configurationOrders) Get(_ context.Context, id string) (payments.Order, error) {
	return s.orders[id], nil
}

type configurationQuotes struct {
	payments.Core
	next int64
}

func (c *configurationQuotes) Prepare(_ context.Context, _ string, r payments.PrepareRequest) (payments.Intent, error) {
	c.next++
	return payments.Intent{ID: c.next, AccountID: r.AccountID, CreatedBy: 42, Key: r.Key, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: r.Currency, AmountMinor: r.AmountMinor, CreditUnits: 1000, CreditUnit: "credit_500k_usd", RequestHash: make([]byte, 32)}, nil
}

func adminPrivateKey(t *testing.T) string {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)}))
}

func pancakeManagementFixture(t *testing.T) (*PancakeAdministration, *payments.Service, *configurationOrders, *configurationQuotes) {
	t.Helper()
	key := paymentPrivate(t, "merchant.pem", adminPrivateKey(t))
	document := multiPaymentDocument{Version: 2, Channels: []multiPaymentChannel{
		{ID: 51, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "USD", Enabled: true, PrivateKeyFile: key, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", TaxCategory: "saas", ReturnURL: "https://example.test/old"},
		{ID: 53, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl", Environment: "test", Currency: "CNY", Enabled: true, PrivateKeyFile: key, StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl", TaxCategory: "saas", ReturnURL: "https://example.test/old"},
	}}
	var raw bytes.Buffer
	if err := toml.NewEncoder(&raw).Encode(document); err != nil {
		t.Fatal(err)
	}
	path := paymentPrivate(t, "payments.toml", raw.String())
	adapters, err := loadMultiPaymentAdapters(raw.Bytes(), nil)
	if err != nil {
		t.Fatal(err)
	}
	orders, quotes := &configurationOrders{orders: map[string]payments.Order{}}, &configurationQuotes{}
	wallet, err := payments.New(orders, quotes, adapters...)
	if err != nil {
		t.Fatal(err)
	}
	admin, err := newPancakeAdministration(path, wallet, nil)
	if err != nil {
		t.Fatal(err)
	}
	return admin, wallet, orders, quotes
}

func configurationOrder(t *testing.T, wallet *payments.Service, key string) payments.Order {
	t.Helper()
	order, err := wallet.Create(context.Background(), "session", payments.PrepareRequest{Key: key, AccountID: 42, ChannelID: 51, Currency: "USD", AmountMinor: 100})
	if err != nil {
		t.Fatal(err)
	}
	return order
}

func TestPancakeManagementSavesPrivateConfigurationAndRestarts(t *testing.T) {
	admin, wallet, orders, quotes := pancakeManagementFixture(t)
	before := configurationOrder(t, wallet, "configuration_before_01")
	oldKeyFile := admin.document.Channels[0].PrivateKeyFile
	request := payments.PancakeAdminRequest{StoreID: before.PancakeBinding.StoreID, ProductID: before.PancakeBinding.ProductID, ReturnURL: "https://example.test/new", PrivateKey: adminPrivateKey(t)}
	value, err := admin.Save(context.Background(), request)
	if err != nil || value == nil {
		t.Fatal("save failed", err)
	}
	if quotes.next != 1 {
		t.Fatal("configuration save performed financial preparation")
	}
	current := configurationOrder(t, wallet, "configuration_after_01")
	if current.PancakeBinding.ReturnURL != request.ReturnURL || before.PancakeBinding.ReturnURL != "https://example.test/old" || orders.orders[before.ID].PancakeBinding.ReturnURL != before.PancakeBinding.ReturnURL {
		t.Fatal("new or frozen order binding changed incorrectly")
	}
	raw, err := os.ReadFile(admin.path)
	if err != nil || bytes.Contains(raw, []byte("PRIVATE KEY")) {
		t.Fatal("configuration leaked inline private key")
	}
	var saved multiPaymentDocument
	if _, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&saved); err != nil {
		t.Fatal(err)
	}
	for _, channel := range saved.Channels {
		info, err := os.Stat(channel.PrivateKeyFile)
		if err != nil || info.Mode().Perm() != 0600 || channel.PrivateKeyFile == oldKeyFile || channel.ReturnURL != request.ReturnURL {
			t.Fatal("aliases did not persist one private key and return target")
		}
	}
	if _, err := os.Stat(oldKeyFile); err != nil {
		t.Fatal("old credential was destroyed")
	}
	restored, err := loadMultiPaymentAdapters(raw, nil)
	if err != nil {
		t.Fatal("saved config not restartable", err)
	}
	restarted, err := payments.New(orders, quotes, restored...)
	if err != nil {
		t.Fatal(err)
	}
	if got := configurationOrder(t, restarted, "configuration_restart_01"); !reflect.DeepEqual(got.PancakeBinding, current.PancakeBinding) {
		t.Fatal("restart lost selected configuration")
	}
}

func TestPancakeManagementPreservesPolicyAndFailedSaves(t *testing.T) {
	admin, wallet, _, _ := pancakeManagementFixture(t)
	old := append([]byte(nil), admin.expected...)
	request := payments.PancakeAdminRequest{StoreID: admin.document.Channels[0].StoreID, ProductID: admin.document.Channels[0].ProductID, ReturnURL: "https://example.test/new"}
	for _, field := range []string{"merchant", "store", "product"} {
		bad := request
		switch field {
		case "merchant":
			bad.MerchantID = "MER_abcdefghijkl0123456789"
		case "store":
			bad.StoreID = "STO_abcdefghijkl0123456789"
		case "product":
			bad.ProductID = "PROD_abcdefghijkl0123456789"
		}
		if _, err := admin.Save(context.Background(), bad); !errors.Is(err, payments.ErrPancakePolicy) {
			t.Fatal("Core binding changed through Go", field, err)
		}
	}
	admin.replace = func(string, []byte, []byte) error { return errors.New("disk unavailable") }
	if _, err := admin.Save(context.Background(), request); !errors.Is(err, payments.ErrUnavailable) {
		t.Fatal(err)
	}
	if got := configurationOrder(t, wallet, "configuration_failure_01"); got.PancakeBinding.ReturnURL != "https://example.test/old" {
		t.Fatal("failed save changed live configuration")
	}
	if raw, _ := os.ReadFile(admin.path); !bytes.Equal(raw, old) {
		t.Fatal("rejected save changed disk")
	}
	admin.replace = replacePancakeConfiguration
	if err := os.WriteFile(admin.path, append(old, []byte("\n# operator change\n")...), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := admin.Save(context.Background(), request); !errors.Is(err, payments.ErrConflict) {
		t.Fatal("operator edit overwritten", err)
	}
}

func TestPancakeManagementFollowsVisibleUncertainCommit(t *testing.T) {
	admin, wallet, _, _ := pancakeManagementFixture(t)
	admin.replace = func(path string, previous, next []byte) error {
		return replacePancakeConfigurationWithSync(path, previous, next, func(*os.File) error { return errors.New("directory sync unavailable") })
	}
	request := payments.PancakeAdminRequest{StoreID: admin.document.Channels[0].StoreID, ProductID: admin.document.Channels[0].ProductID, ReturnURL: "https://example.test/uncertain", PrivateKey: adminPrivateKey(t)}
	_, err := admin.Save(context.Background(), request)
	var committed *payments.ConfigurationCommittedError
	if !errors.As(err, &committed) {
		t.Fatal("uncertain durability was hidden", err)
	}
	if got := configurationOrder(t, wallet, "configuration_uncertain_01"); got.PancakeBinding.ReturnURL != request.ReturnURL {
		t.Fatal("runtime did not follow visible config")
	}
	if _, err := admin.selected(); err != nil {
		t.Fatal("visible config was not read back into admin state", err)
	}
	if _, err := os.Stat(admin.document.Channels[0].PrivateKeyFile); err != nil {
		t.Fatal("committed key removed after uncertainty")
	}
}

func TestPancakeManagementSerializesConcurrentSaves(t *testing.T) {
	admin, wallet, _, _ := pancakeManagementFixture(t)
	request := payments.PancakeAdminRequest{StoreID: admin.document.Channels[0].StoreID, ProductID: admin.document.Channels[0].ProductID}
	var wg sync.WaitGroup
	failures := make(chan error, 2)
	for _, target := range []string{"https://example.test/first", "https://example.test/second"} {
		wg.Add(1)
		go func(target string) {
			defer wg.Done()
			r := request
			r.ReturnURL = target
			_, err := admin.Save(context.Background(), r)
			failures <- err
		}(target)
	}
	wg.Wait()
	close(failures)
	for err := range failures {
		if err != nil {
			t.Fatal(err)
		}
	}
	var saved multiPaymentDocument
	if _, err := toml.DecodeFile(admin.path, &saved); err != nil {
		t.Fatal(err)
	}
	order := configurationOrder(t, wallet, "configuration_concurrent_01")
	if saved.Channels[0].ReturnURL != order.PancakeBinding.ReturnURL || saved.Channels[1].ReturnURL != order.PancakeBinding.ReturnURL {
		t.Fatal("concurrent writes split disk, alias or runtime state")
	}
}
