package assistant_catalog

import (
	"context"
	"encoding/json"
	"net/url"
	"strings"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type productArguments struct {
	Query  string `json:"query"`
	Offset *int   `json:"offset"`
	Limit  *int   `json:"limit"`
}
type productIDArguments struct {
	ProductID string `json:"product_id"`
}

// These DTOs are deliberately smaller than the merchant/domain records. Fixed
// delivery contents, moderation notes, receipts and credential fields have no
// representable output field. Unknown stock or non-credit prices stay null.
type productVariant struct {
	ID                 string       `json:"id"`
	Name               string       `json:"name"`
	PriceQuota         *int64       `json:"price_quota"`
	Price              *store.Price `json:"price,omitempty"`
	Template           string       `json:"template,omitempty"`
	Enabled            bool         `json:"enabled"`
	UnlimitedSupply    *bool        `json:"unlimited_supply,omitempty"`
	InventoryAvailable *int64       `json:"inventory_available,omitempty"`
	SaleAvailable      *int64       `json:"sale_available,omitempty"`
	TradingPaused      bool         `json:"trading_paused"`
	Tradable           bool         `json:"tradable"`
}
type productView struct {
	EntityKind            string           `json:"entity_kind"`
	ID                    string           `json:"id"`
	Title                 string           `json:"title"`
	Description           string           `json:"description"`
	DescriptionTruncated  bool             `json:"description_truncated"`
	RecordKind            string           `json:"record_kind"`
	Status                string           `json:"status"`
	Visibility            string           `json:"visibility"`
	PriceQuota            *int64           `json:"price_quota"`
	PriceMinQuota         *int64           `json:"price_min_quota,omitempty"`
	PriceMaxQuota         *int64           `json:"price_max_quota,omitempty"`
	AvailableStock        *int64           `json:"available_stock,omitempty"`
	SaleAvailable         *int64           `json:"sale_available,omitempty"`
	PaymentMethods        []string         `json:"payment_methods"`
	TradingPaused         bool             `json:"trading_paused"`
	TestMode              bool             `json:"test_mode"`
	PurchaseLoginRequired bool             `json:"purchase_login_required"`
	PickupLoginRequired   bool             `json:"pickup_login_required"`
	PickupCodeRequired    bool             `json:"pickup_code_required"`
	MaxQuantityPerOrder   *int64           `json:"max_quantity_per_order,omitempty"`
	MaxQuantityPerBuyer   *int64           `json:"max_quantity_per_buyer,omitempty"`
	Variants              []productVariant `json:"variants"`
	VariantStockKnown     bool             `json:"variant_stock_known"`
	VariantCount          int              `json:"variant_count"`
	VariantsTruncated     bool             `json:"variants_truncated"`
	Tradable              bool             `json:"tradable"`
	Href                  string           `json:"href"`
	PriceUnit             string           `json:"price_unit"`
	CreditsPerUSD         *int64           `json:"credits_per_usd,omitempty"`
}
type productsResult struct {
	OK         bool          `json:"ok"`
	EntityKind string        `json:"entity_kind"`
	Items      []productView `json:"items"`
	Offset     int           `json:"offset"`
	Limit      int           `json:"limit"`
	HasMore    bool          `json:"has_more"`
}
type productResult struct {
	OK      bool        `json:"ok"`
	Product productView `json:"product"`
}

func clipped(s string, max int) (string, bool) {
	r := []rune(s)
	if len(r) > max {
		return string(r[:max]), true
	}
	return s, false
}
func productProjection(value map[string]any, full bool) (productView, error) {
	var v productView
	raw, err := json.Marshal(value)
	if err != nil || json.Unmarshal(raw, &v) != nil || !access.ID(v.ID) || !utf8.ValidString(v.Title) || len(v.Title) == 0 || len([]rune(v.Title)) > 200 {
		return v, access.ErrUnavailable
	}
	maxDescription := 1200
	if full {
		maxDescription = 16000
	}
	v.Description, v.DescriptionTruncated = clipped(v.Description, maxDescription)
	v.EntityKind = "store_product"
	v.Href = "/store/products/" + url.PathEscape(v.ID)
	if v.TestMode {
		v.Href += "?owner_preview=true"
	}
	v.PriceUnit = "CREDIT"
	credits := int64(500000)
	v.CreditsPerUSD = &credits
	// Native catalogue prices carry their original unit. A currency price does
	// not acquire a credit value or exchange rate from the assistant.
	if v.RecordKind == "native" && v.PriceQuota == nil {
		v.PriceUnit = "native_variant_units"
		v.CreditsPerUSD = nil
	}
	if v.RecordKind != "legacy" && v.RecordKind != "native" {
		return v, access.ErrUnavailable
	}
	for _, amount := range []*int64{v.PriceQuota, v.PriceMinQuota, v.PriceMaxQuota} {
		if amount != nil && *amount < 0 {
			return v, access.ErrUnavailable
		}
	}
	if v.RecordKind == "legacy" && v.PriceQuota == nil {
		return v, access.ErrUnavailable
	}
	if v.PaymentMethods == nil {
		v.PaymentMethods = []string{}
	}
	active := v.Status == "published" || v.TestMode && (v.Status == "draft" || v.Status == "pending")
	v.Tradable = active && !v.TradingPaused && v.AvailableStock != nil && *v.AvailableStock > 0 && len(v.PaymentMethods) > 0
	if v.SaleAvailable != nil && *v.SaleAvailable <= 0 {
		v.Tradable = false
	}
	rows := make([]productVariant, 0, len(v.Variants))
	v.VariantStockKnown = true
	for _, row := range v.Variants {
		if !row.Enabled {
			continue
		}
		if !access.ID(row.ID) || row.Name == "" || len([]rune(row.Name)) > 200 || row.PriceQuota != nil && *row.PriceQuota < 0 {
			return v, access.ErrUnavailable
		}
		if row.PriceQuota == nil && row.Price == nil {
			return v, access.ErrUnavailable
		}
		if row.Price != nil && row.Price.Unit == "" && row.Price.Currency == "" {
			return v, access.ErrUnavailable
		}
		if row.Price != nil && row.Price.Unit == "credits" && (row.PriceQuota == nil || row.Price.Amount != *row.PriceQuota) {
			return v, access.ErrUnavailable
		}
		if row.Price != nil && row.Price.Unit != "credits" && row.PriceQuota != nil {
			return v, access.ErrUnavailable
		}
		known := row.SaleAvailable != nil || row.InventoryAvailable != nil
		v.VariantStockKnown = v.VariantStockKnown && known
		row.Tradable = v.Tradable && known && !row.TradingPaused
		if row.InventoryAvailable != nil && *row.InventoryAvailable <= 0 || row.SaleAvailable != nil && *row.SaleAvailable <= 0 {
			row.Tradable = false
		}
		rows = append(rows, row)
	}
	v.VariantCount = len(rows)
	v.VariantsTruncated = len(rows) > 50
	if len(rows) > 50 {
		rows = rows[:50]
	}
	v.Variants = rows
	if len(rows) == 0 {
		v.VariantStockKnown = false
	}
	return v, nil
}
func (t *tool) products(ctx context.Context, in assistant.AgentInvocation, p access.Principal) (any, error) {
	if access.Missing(t.executor.store) {
		return nil, access.ErrUnavailable
	}
	if t.definition.Name == "get_store_product" {
		var a productIDArguments
		if access.Decode(in.Call.Arguments, &a) != nil || !access.ID(a.ProductID) {
			return nil, access.ErrInvalid
		}
		row, err := t.executor.store.ProductForViewer(ctx, in.Credential, p.UserID, a.ProductID)
		if err != nil {
			return nil, err
		}
		v, err := productProjection(row, true)
		if err != nil {
			return nil, err
		}
		if v.ID != a.ProductID {
			return nil, access.ErrConflict
		}
		return productResult{true, v}, nil
	}
	var a productArguments
	if access.Decode(in.Call.Arguments, &a) != nil {
		return nil, access.ErrInvalid
	}
	a.Query = strings.TrimSpace(a.Query)
	offset, limit := 0, 10
	if a.Offset != nil {
		offset = *a.Offset
	}
	if a.Limit != nil {
		limit = *a.Limit
	}
	if !utf8.ValidString(a.Query) || len(a.Query) > 200 || utf8.RuneCountInString(a.Query) > 120 || offset < 0 || offset > 10000 || limit < 1 || limit > 20 {
		return nil, access.ErrInvalid
	}
	rows, err := t.executor.store.CatalogueForViewer(ctx, in.Credential, p.UserID, store.CatalogueQuery{Text: a.Query, Offset: offset, Limit: limit})
	if err != nil {
		return nil, err
	}
	if len(rows) > limit {
		return nil, access.ErrUnavailable
	}
	out := productsResult{OK: true, EntityKind: "store_product", Items: []productView{}, Offset: offset, Limit: limit, HasMore: len(rows) == limit}
	seen := map[string]bool{}
	for _, row := range rows {
		v, err := productProjection(row, false)
		if err != nil {
			return nil, err
		}
		if seen[v.ID] {
			return nil, access.ErrConflict
		}
		seen[v.ID] = true
		out.Items = append(out.Items, v)
	}
	return out, nil
}
