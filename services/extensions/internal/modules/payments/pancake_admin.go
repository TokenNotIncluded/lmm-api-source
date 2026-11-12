package payments

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strings"
	"time"

	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

var ErrPancakePolicy = errors.New("Pancake binding requires a Core payment policy transition")

// PancakeAdminRequest keeps credentials in the private request body. Channel
// identity is selected from installation configuration, never monetary input.
type PancakeAdminRequest struct {
	MerchantID string `json:"merchant_id"`
	PrivateKey string `json:"private_key"`
	ReturnURL  string `json:"return_url"`
	StoreID    string `json:"store_id"`
	ProductID  string `json:"product_id"`
}

type PancakeCatalogProduct struct {
	ID            string `json:"id"`
	Name          string `json:"name"`
	Status        string `json:"status"`
	BillingPeriod string `json:"billingPeriod,omitempty"`
}

type PancakeCatalogStore struct {
	ID                   string                  `json:"id"`
	Name                 string                  `json:"name"`
	Status               string                  `json:"status"`
	ProdEnabled          bool                    `json:"prodEnabled"`
	OnetimeProducts      []PancakeCatalogProduct `json:"onetimeProducts"`
	SubscriptionProducts []PancakeCatalogProduct `json:"subscriptionProducts"`
}

type PancakeCatalog struct {
	Stores []PancakeCatalogStore `json:"stores"`
}

type PancakePairResult struct {
	StoreID     string `json:"store_id"`
	StoreName   string `json:"store_name"`
	ProductID   string `json:"product_id,omitempty"`
	ProductName string `json:"product_name,omitempty"`
	OrphanStore bool   `json:"orphan_store,omitempty"`
}

func (p *Pancake) AdminCatalog(ctx context.Context) (*PancakeCatalog, error) {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	stores, err := pancake.GraphQLQuery[PancakeCatalog](ctx, p.client, pancake.GraphQLParams{Query: `query { stores { id name status prodEnabled } }`})
	if err != nil || stores == nil || len(stores.Errors) != 0 {
		return nil, ErrUnavailable
	}
	result := stores.Data
	if result.Stores == nil {
		result.Stores = []PancakeCatalogStore{}
	}
	for i := range result.Stores {
		s := &result.Stores[i]
		if !validID(s.ID) {
			return nil, ErrConflict
		}
		products, err := pancake.GraphQLQuery[struct {
			Onetime      []PancakeCatalogProduct `json:"onetimeProducts"`
			Subscription []PancakeCatalogProduct `json:"subscriptionProducts"`
		}](ctx, p.client, pancake.GraphQLParams{
			Query: `query ($storeId: String!) {
				onetimeProducts(storeId: $storeId, filter: { status: { eq: "active" } }) { id name status }
				subscriptionProducts(storeId: $storeId, filter: { status: { eq: "active" } }) { id name status billingPeriod }
			}`,
			Variables: map[string]any{"storeId": s.ID},
		})
		if err != nil || products == nil || len(products.Errors) != 0 {
			return nil, ErrUnavailable
		}
		s.OnetimeProducts = activePancakeProducts(products.Data.Onetime)
		s.SubscriptionProducts = activePancakeProducts(products.Data.Subscription)
	}
	return &result, nil
}

func activePancakeProducts(products []PancakeCatalogProduct) []PancakeCatalogProduct {
	active := make([]PancakeCatalogProduct, 0, len(products))
	for _, p := range products {
		if p.Status == "active" {
			active = append(active, p)
		}
	}
	return active
}

// AdminPair prepares provider objects only. The installation's Core policy must
// admit the selected binding before it can replace an active payment channel.
func (p *Pancake) AdminPair(ctx context.Context, returnURL string, currencies []string) (*PancakePairResult, error) {
	if returnURL != "" {
		if _, err := secureURL(returnURL); err != nil {
			return nil, ErrInvalid
		}
	}
	prices := pancake.Prices{}
	for _, currency := range currencies {
		if currency != "USD" && currency != "CNY" {
			return nil, ErrInvalid
		}
		// A seed price enables the supported currency. Every wallet checkout
		// overrides it with the exact immutable amount prepared by Core.
		prices[currency] = pancake.PriceInfo{Amount: "1.00", TaxCategory: pancake.TaxCategory(p.cfg.TaxCategory)}
	}
	if len(prices) == 0 {
		return nil, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	input, _ := json.Marshal([]any{p.cfg.Channel.Merchant, p.cfg.Channel.Environment, returnURL, prices})
	digest := sha256.Sum256(input)
	key := hex.EncodeToString(digest[:])
	created, err := p.client.Stores.Create(ctx, pancake.CreateStoreParams{Name: "LMM API"}, pancake.WithIdempotencyKey("lmm-store-"+key))
	if err != nil || created == nil || !validID(created.Store.ID) {
		return nil, ErrUnavailable
	}
	result := &PancakePairResult{StoreID: created.Store.ID, StoreName: created.Store.Name, OrphanStore: true}
	params := pancake.CreateOnetimeProductParams{StoreID: result.StoreID, Name: "LMM API wallet credits", Prices: prices}
	if returnURL != "" {
		params.SuccessURL = &returnURL
	}
	product, err := p.client.OnetimeProducts.Create(ctx, params, pancake.WithIdempotencyKey("lmm-product-"+key))
	if err != nil || product == nil || !validID(product.Product.ID) || product.Product.StoreID != result.StoreID {
		return result, ErrUnavailable
	}
	result.ProductID, result.ProductName = product.Product.ID, product.Product.Name
	if p.cfg.Channel.Environment == "live" {
		published, err := p.client.OnetimeProducts.Publish(ctx, pancake.PublishOnetimeProductParams{ID: result.ProductID}, pancake.WithIdempotencyKey("lmm-publish-"+key))
		if err != nil || published == nil || published.Product.ID != result.ProductID || published.Product.StoreID != result.StoreID || published.Product.Status != pancake.ProductVersionStatusActive {
			return result, ErrUnavailable
		}
	}
	result.OrphanStore = false
	return result, nil
}

func NormalizePancakeAdminRequest(r PancakeAdminRequest) PancakeAdminRequest {
	r.MerchantID, r.PrivateKey = strings.TrimSpace(r.MerchantID), strings.TrimSpace(r.PrivateKey)
	r.StoreID, r.ProductID, r.ReturnURL = strings.TrimSpace(r.StoreID), strings.TrimSpace(r.ProductID), strings.TrimSpace(r.ReturnURL)
	return r
}
