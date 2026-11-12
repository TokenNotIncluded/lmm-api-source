package composition

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

type PancakeAdministration struct {
	mu       sync.Mutex
	path     string
	expected []byte
	document multiPaymentDocument
	runtime  *payments.PancakeConfiguration
	replace  func(string, []byte, []byte) error
}

func newPancakeAdministration(path string, wallet *payments.Service, subscriptions *payments.PancakeSubscriptions) (*PancakeAdministration, error) {
	raw, err := coreclient.ReadPrivateFile(path, 64<<10)
	if err != nil {
		return nil, payments.ErrUnavailable
	}
	var document multiPaymentDocument
	meta, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&document)
	if err != nil || len(meta.Undecoded()) != 0 || document.Version != 2 {
		return nil, payments.ErrInvalid
	}
	return &PancakeAdministration{path: path, expected: raw, document: document, runtime: payments.AttachPancakeConfiguration(wallet, subscriptions), replace: replacePancakeConfiguration}, nil
}

// The legacy form describes one merchant binding. Currency aliases share it;
// multiple distinct bindings require the caller to use explicit channel setup.
func (a *PancakeAdministration) selected() ([]int, error) {
	current, err := coreclient.ReadPrivateFile(a.path, 64<<10)
	if err != nil || !bytes.Equal(current, a.expected) {
		return nil, payments.ErrConflict
	}
	var selected []int
	for i, c := range a.document.Channels {
		if c.Provider == "waffo_pancake" && c.Enabled {
			selected = append(selected, i)
		}
	}
	if len(selected) == 0 {
		for i, c := range a.document.Channels {
			if c.Provider == "waffo_pancake" {
				selected = append(selected, i)
			}
		}
	}
	if len(selected) == 0 {
		return nil, payments.ErrUnavailable
	}
	first := a.document.Channels[selected[0]]
	for _, i := range selected[1:] {
		c := a.document.Channels[i]
		if c.Merchant != first.Merchant || c.Environment != first.Environment || c.StoreID != first.StoreID || c.ProductID != first.ProductID || c.PrivateKeyFile != first.PrivateKeyFile || c.TaxCategory != first.TaxCategory {
			return nil, payments.ErrConflict
		}
	}
	return selected, nil
}

func pancakeAdminAdapter(c multiPaymentChannel, request payments.PancakeAdminRequest) (*payments.Pancake, error) {
	merchant, key := c.Merchant, request.PrivateKey
	if request.MerchantID != "" {
		merchant = request.MerchantID
	}
	if key == "" {
		var err error
		key, err = outboundPaymentSecret(c.PrivateKeyFile, true)
		if err != nil {
			return nil, payments.ErrUnavailable
		}
	}
	return payments.NewPancake(payments.PancakeConfig{
		Channel:    payments.Channel{ID: c.ID, Provider: c.Provider, Merchant: merchant, Environment: c.Environment, Currency: c.Currency, Enabled: c.Enabled, CreditQuantity: c.CreditQuantity},
		PrivateKey: key, StoreID: c.StoreID, ProductID: c.ProductID, ReturnURL: c.ReturnURL,
		CheckoutRegion: c.CheckoutRegion, CheckoutLanguage: c.CheckoutLanguage, TaxCategory: c.TaxCategory,
	})
}

func (a *PancakeAdministration) probe(request payments.PancakeAdminRequest) (*payments.Pancake, []string, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	selected, err := a.selected()
	if err != nil {
		return nil, nil, err
	}
	adapter, err := pancakeAdminAdapter(a.document.Channels[selected[0]], request)
	if err != nil {
		return nil, nil, err
	}
	currencies := make([]string, 0, len(selected))
	for _, i := range selected {
		currencies = append(currencies, a.document.Channels[i].Currency)
	}
	return adapter, currencies, nil
}

func (a *PancakeAdministration) Catalog(ctx context.Context, request payments.PancakeAdminRequest) (any, error) {
	adapter, _, err := a.probe(payments.NormalizePancakeAdminRequest(request))
	if err != nil {
		return nil, err
	}
	return adapter.AdminCatalog(ctx)
}

func (a *PancakeAdministration) Pair(ctx context.Context, request payments.PancakeAdminRequest) (any, error) {
	request = payments.NormalizePancakeAdminRequest(request)
	adapter, currencies, err := a.probe(request)
	if err != nil {
		return nil, err
	}
	return adapter.AdminPair(ctx, request.ReturnURL, currencies)
}

func (a *PancakeAdministration) Save(ctx context.Context, request payments.PancakeAdminRequest) (any, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if ctx.Err() != nil {
		return nil, payments.ErrUnavailable
	}
	request = payments.NormalizePancakeAdminRequest(request)
	selected, err := a.selected()
	if err != nil {
		return nil, err
	}
	first := a.document.Channels[selected[0]]
	if request.StoreID == "" || request.ProductID == "" {
		return nil, payments.ErrInvalid
	}
	// Go must not make a merchant/product active while Core still verifies the
	// old binding. Core's separate policy transition is required for those IDs.
	if request.MerchantID != "" && request.MerchantID != first.Merchant || request.StoreID != first.StoreID || request.ProductID != first.ProductID {
		return nil, payments.ErrPancakePolicy
	}
	next := a.document
	next.Channels = append([]multiPaymentChannel(nil), a.document.Channels...)
	var raw []byte
	var stagedKey string
	err = a.runtime.Update(func() ([]payments.Adapter, map[int64]bool, error) {
		adapters := make([]payments.Adapter, 0, len(selected))
		admission := make(map[int64]bool, len(selected))
		for _, i := range selected {
			next.Channels[i].ReturnURL = request.ReturnURL
			adapter, err := pancakeAdminAdapter(next.Channels[i], request)
			if err != nil {
				return nil, nil, err
			}
			adapters = append(adapters, adapter)
			admission[next.Channels[i].ID] = next.Channels[i].SubscriptionEnabled
		}
		return adapters, admission, nil
	}, func() error {
		if request.PrivateKey != "" {
			stagedKey, err = writePancakePrivateKey(filepath.Dir(a.path), request.PrivateKey)
			if err != nil {
				return payments.ErrUnavailable
			}
			for _, i := range selected {
				next.Channels[i].PrivateKeyFile = stagedKey
			}
		}
		var encoded bytes.Buffer
		if err := toml.NewEncoder(&encoded).Encode(next); err != nil {
			return payments.ErrInvalid
		}
		raw = encoded.Bytes()
		return a.replace(a.path, a.expected, raw)
	})
	var committed *payments.ConfigurationCommittedError
	if err != nil && !errors.As(err, &committed) {
		if stagedKey != "" {
			_ = os.Remove(stagedKey)
		}
		if errors.Is(err, payments.ErrInvalid) || errors.Is(err, payments.ErrConflict) {
			return nil, err
		}
		return nil, payments.ErrUnavailable
	}
	a.expected, a.document = raw, next
	value := map[string]string{"store_id": first.StoreID, "product_id": first.ProductID}
	return value, err
}

// A new immutable key file is staged before the configuration references it.
// Old key files stay intact for other channels and operator-controlled recovery.
func writePancakePrivateKey(directory, key string) (string, error) {
	file, err := os.CreateTemp(directory, ".pancake-key-*")
	if err != nil {
		return "", err
	}
	name := file.Name()
	defer file.Close()
	committed := false
	defer func() {
		if !committed {
			_ = os.Remove(name)
		}
	}()
	if _, err = file.WriteString(key + "\n"); err != nil {
		return "", err
	}
	if err = file.Sync(); err != nil {
		return "", err
	}
	if err = file.Close(); err != nil {
		return "", err
	}
	dir, err := os.Open(directory)
	if err != nil {
		return "", err
	}
	defer dir.Close()
	if err = dir.Sync(); err != nil {
		return "", err
	}
	committed = true
	return name, nil
}
