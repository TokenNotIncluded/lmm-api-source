package store

import (
	"context"
	"encoding/json"
	"errors"
	"math/big"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

const (
	extoreProductKind   = "extore_products"
	extoreCreditsPerUSD = int64(500000)
	extoreMaxVariants   = 30
)

type extoreProductRow struct {
	ID           string            `json:"id"`
	ConnectionID string            `json:"connection_id"`
	SourceID     string            `json:"source_id"`
	ProductID    string            `json:"product_id"`
	Revision     string            `json:"revision"`
	VariantIDs   map[string]string `json:"variant_ids"`
	CardSummary  json.RawMessage   `json:"card_summary,omitempty"`
	LastSyncedAt int64             `json:"last_synced_at"`
}

type ExtoreSyncResult struct {
	SourceID     string `json:"source_id"`
	ProductID    string `json:"product_id,omitempty"`
	Success      bool   `json:"success"`
	Message      string `json:"message"`
	LastSyncedAt int64  `json:"last_synced_at,omitempty"`
}

type extoreVariantIn struct {
	SourceID string
	Name     string
	Price    int64
	Template string
	Enabled  bool
	Fixed    *string
}

func extoreConnectionAAD(id string) string {
	return extorePurpose + "\x00connection\x00" + id
}

func (s *Service) listKind(ctx context.Context, shop, kind string) ([]json.RawMessage, error) {
	var out []json.RawMessage
	after := ""
	for page := 0; page < 100; page++ {
		var batch []json.RawMessage
		err := s.repo.Within(ctx, shop, func(tx Tx) error {
			rows, err := tx.List(kind, after, 100)
			if err != nil {
				return err
			}
			batch = rows
			return nil
		})
		if err != nil {
			return nil, err
		}
		out = append(out, batch...)
		if len(batch) < 100 {
			return out, nil
		}
		var last map[string]any
		if json.Unmarshal(batch[len(batch)-1], &last) != nil {
			return nil, ErrUnavailable
		}
		id, _ := last["id"].(string)
		if id == "" || id <= after {
			return nil, ErrUnavailable
		}
		after = id
	}
	return out, nil
}

func (s *Service) openExtoreCredentials(row extoreConnectionRow) (extore.Credentials, error) {
	if row.RefreshPending || row.ExpiresAt <= time.Now().UTC().Unix() {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	secret, err := s.extore.open(extoreConnectionAAD(row.ID), row.Ciphertext)
	if err != nil {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	var credentials extore.Credentials
	if json.Unmarshal(secret, &credentials) != nil || credentials.Access == "" || credentials.Refresh == "" {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	return credentials, nil
}

func (s *Service) loadExtoreConnection(ctx context.Context, userID int64, id string) (extoreConnectionRow, extore.Credentials, error) {
	shop := extoreSellerShop(userID)
	var row extoreConnectionRow
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		return tx.Get(extoreConnectionKind, id, &row)
	})
	if err != nil {
		return extoreConnectionRow{}, extore.Credentials{}, err
	}
	credentials, err := s.openExtoreCredentials(row)
	return row, credentials, err
}

func (s *Service) refreshExtoreCredentials(ctx context.Context, userID int64, row *extoreConnectionRow, credentials extore.Credentials) (extore.Credentials, error) {
	shop := extoreSellerShop(userID)
	version := row.Version
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		var current extoreConnectionRow
		if err := tx.Get(extoreConnectionKind, row.ID, &current); err != nil {
			return err
		}
		if current.RefreshPending || current.Version != version {
			return ErrExtoreReauthorize
		}
		current.RefreshPending = true
		*row = current
		return tx.Put(extoreConnectionKind, row.ID, current)
	})
	if err != nil {
		return extore.Credentials{}, err
	}
	client := extore.Client{HTTP: s.extoreHTTP}
	next, err := client.RefreshConnection(ctx, row.Origin, row.ClientID, credentials)
	if err != nil {
		return extore.Credentials{}, ErrExtoreReauthorize
	}
	secret, err := json.Marshal(next)
	if err != nil {
		return extore.Credentials{}, ErrUnavailable
	}
	cipher, err := s.extore.seal(extoreConnectionAAD(row.ID), secret)
	if err != nil {
		return extore.Credentials{}, ErrUnavailable
	}
	err = s.repo.Within(ctx, shop, func(tx Tx) error {
		var current extoreConnectionRow
		if err := tx.Get(extoreConnectionKind, row.ID, &current); err != nil {
			return err
		}
		if !current.RefreshPending || current.Version != version {
			return ErrExtoreReauthorize
		}
		current.Ciphertext = cipher
		current.RefreshPending = false
		current.ExpiresAt = int64(next.GrantExpires)
		current.Version = version + 1
		current.UpdatedAt = time.Now().UTC().Unix()
		*row = current
		return tx.Put(extoreConnectionKind, row.ID, current)
	})
	if err != nil {
		return extore.Credentials{}, err
	}
	return *next, nil
}

func (s *Service) connectionCatalog(ctx context.Context, userID int64, id string) (*extore.Catalog, extoreConnectionRow, error) {
	row, credentials, err := s.loadExtoreConnection(ctx, userID, id)
	if err != nil {
		return nil, extoreConnectionRow{}, err
	}
	if credentials.AccessExpires <= time.Now().Unix()+30 {
		credentials, err = s.refreshExtoreCredentials(ctx, userID, &row, credentials)
		if err != nil {
			return nil, extoreConnectionRow{}, err
		}
	}
	client := extore.Client{HTTP: s.extoreHTTP}
	catalog, err := client.ConnectionCatalog(ctx, row.Origin, credentials)
	if err != nil {
		return nil, extoreConnectionRow{}, mapExtoreErr(err)
	}
	return catalog, row, nil
}

// ExtoreConnections lists durable shop connections and already synchronized sources.
func (s *Service) ExtoreConnections(ctx context.Context, token string, userID int64) (map[string]any, error) {
	if !s.ExtoreReady() {
		return nil, ErrUnavailable
	}
	if userID <= 0 || !Text(token, 256) {
		return nil, ErrUnauthorized
	}
	if _, err := s.check(ctx, token, Account{Kind: "personal", ID: userID}, Read); err != nil {
		return nil, err
	}
	extoreShop := extoreSellerShop(userID)
	connectionRows, err := s.listKind(ctx, extoreShop, extoreConnectionKind)
	if err != nil {
		return nil, err
	}
	connections := make([]map[string]any, 0, len(connectionRows))
	for _, raw := range connectionRows {
		var row extoreConnectionRow
		if json.Unmarshal(raw, &row) != nil || row.ID == "" {
			return nil, ErrUnavailable
		}
		connections = append(connections, map[string]any{
			"id": row.ID, "origin": row.Origin, "shop_id": row.ShopID, "shop_name": row.ShopName,
			"expires_at": row.ExpiresAt, "updated_at": row.UpdatedAt,
		})
	}
	legacy, err := s.legacyShop(ctx, Account{Kind: "personal", ID: userID})
	if err != nil {
		return nil, err
	}
	sourceRows, err := s.listKind(ctx, legacy.ID, extoreProductKind)
	if err != nil {
		return nil, err
	}
	sources := make([]map[string]any, 0, len(sourceRows))
	for _, raw := range sourceRows {
		var row extoreProductRow
		if json.Unmarshal(raw, &row) != nil || row.ProductID == "" {
			return nil, ErrUnavailable
		}
		item := map[string]any{
			"connection_id": row.ConnectionID, "source_id": row.SourceID, "product_id": row.ProductID,
			"revision": row.Revision, "last_synced_at": row.LastSyncedAt,
		}
		if len(row.CardSummary) > 0 {
			item["card_summary"] = json.RawMessage(append([]byte(nil), row.CardSummary...))
		}
		sources = append(sources, item)
	}
	return map[string]any{"connections": connections, "sources": sources}, nil
}

// ExtoreConnectionCatalog refreshes the stored grant when needed and returns the live catalogue.
func (s *Service) ExtoreConnectionCatalog(ctx context.Context, token string, userID int64, id string) (any, error) {
	if !s.ExtoreReady() {
		return nil, ErrUnavailable
	}
	if userID <= 0 || !Text(token, 256) || !Text(id, 128) {
		return nil, ErrUnauthorized
	}
	if _, err := s.check(ctx, token, Account{Kind: "personal", ID: userID}, Read); err != nil {
		return nil, err
	}
	catalog, _, err := s.connectionCatalog(ctx, userID, id)
	if err != nil {
		return nil, err
	}
	return catalog, nil
}

// ExtoreSyncProduct imports one approved Extore listing into the seller catalogue.
func (s *Service) ExtoreSyncProduct(ctx context.Context, token string, userID int64, connectionID, sourceID string) (any, error) {
	if !s.ExtoreReady() {
		return nil, ErrUnavailable
	}
	if userID <= 0 || !Text(token, 256) || !Text(connectionID, 128) || !Text(sourceID, 100) {
		return nil, ErrUnauthorized
	}
	if _, err := s.check(ctx, token, Account{Kind: "personal", ID: userID}, Manage); err != nil {
		return nil, err
	}
	catalog, _, err := s.connectionCatalog(ctx, userID, connectionID)
	if err != nil {
		return nil, err
	}
	for _, raw := range catalog.Products {
		var listing struct {
			ID string `json:"id"`
		}
		if json.Unmarshal(raw, &listing) != nil {
			return nil, mapExtoreErr(extore.ErrProtocol)
		}
		if listing.ID != sourceID {
			continue
		}
		source, err := s.syncExtoreListing(ctx, userID, connectionID, raw)
		if err != nil {
			return nil, err
		}
		return map[string]any{
			"connection_id": source.ConnectionID, "source_id": source.SourceID, "product_id": source.ProductID,
			"revision": source.Revision, "last_synced_at": source.LastSyncedAt, "card_summary": source.CardSummary,
		}, nil
	}
	return nil, ErrForbidden
}

func (s *Service) syncExtoreCatalog(ctx context.Context, userID int64, connectionID string, catalog *extore.Catalog) []ExtoreSyncResult {
	results := make([]ExtoreSyncResult, 0, len(catalog.Products))
	for _, raw := range catalog.Products {
		var listing struct {
			ID string `json:"id"`
		}
		_ = json.Unmarshal(raw, &listing)
		source, err := s.syncExtoreListing(ctx, userID, connectionID, raw)
		result := ExtoreSyncResult{SourceID: listing.ID, Success: err == nil}
		if err != nil {
			result.Message = extoreSyncMessage(err)
		} else {
			result.ProductID = source.ProductID
			result.LastSyncedAt = source.LastSyncedAt
			result.Message = "Product synchronized from Extore"
		}
		results = append(results, result)
	}
	return results
}

func extoreSyncMessage(err error) string {
	switch {
	case errors.Is(err, ErrInvalid):
		return "Check the product and all variant sales settings in Extore"
	case errors.Is(err, ErrConflict):
		return "This product is under review; finish or cancel the review before syncing"
	case errors.Is(err, ErrExtoreReauthorize):
		return "Extore authorization expired or could not be refreshed. Connect again."
	case err != nil && strings.HasPrefix(err.Error(), "Configure the selling price"):
		return "Configure the selling price for every variant in Extore before syncing"
	default:
		return "The product could not be synchronized; try again after checking the connection"
	}
}

func extoreUSDCredits(raw string) (int64, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return 0, ErrInvalid
	}
	r := new(big.Rat)
	if _, ok := r.SetString(raw); !ok {
		return 0, ErrInvalid
	}
	r.Mul(r, big.NewRat(extoreCreditsPerUSD, 1))
	if !r.IsInt() {
		return 0, ErrInvalid
	}
	n := r.Num()
	if !n.IsInt64() {
		return 0, ErrInvalid
	}
	v := n.Int64()
	if v <= 0 || v > legacySafeInteger {
		return 0, ErrInvalid
	}
	return v, nil
}

func extoreDeliveryTemplate(template string, fixed *string) error {
	if stockTemplate(template) {
		if fixed != nil && strings.TrimSpace(*fixed) != "" {
			return ErrInvalid
		}
		return nil
	}
	if template == "fixed-content" {
		if fixed == nil || !Text(*fixed, 16000) {
			return ErrInvalid
		}
		return nil
	}
	return ErrInvalid
}

func (s *Service) syncExtoreListing(ctx context.Context, userID int64, connectionID string, raw json.RawMessage) (*extoreProductRow, error) {
	var listing struct {
		ID            string `json:"id"`
		Revision      string `json:"revision"`
		SyncRevision  string `json:"sync_revision"`
		RedemptionURL string `json:"redemption_url"`
		Product       struct {
			Name         string          `json:"name"`
			Description  string          `json:"description"`
			Logo         string          `json:"logo"`
			Image        string          `json:"image"`
			Public       bool            `json:"public"`
			SupportEmail string          `json:"support_email"`
			Marketplace  json.RawMessage `json:"marketplace"`
		} `json:"product"`
		Variants []struct {
			ID          string  `json:"id"`
			Name        string  `json:"name"`
			Enabled     bool    `json:"enabled"`
			Price       *string `json:"price"`
			Currency    string  `json:"currency"`
			Marketplace struct {
				PriceQuota   *int64  `json:"price_quota"`
				Template     string  `json:"template"`
				FixedContent *string `json:"fixed_content"`
			} `json:"marketplace"`
		} `json:"variants"`
		CardSummary json.RawMessage `json:"card_summary"`
	}
	if json.Unmarshal(raw, &listing) != nil || listing.ID == "" || len(listing.Variants) == 0 || len(listing.Product.Marketplace) == 0 {
		return nil, mapExtoreErr(extore.ErrProtocol)
	}
	var in LegacyProductInput
	if json.Unmarshal(listing.Product.Marketplace, &in) != nil {
		return nil, mapExtoreErr(extore.ErrProtocol)
	}
	var extra struct {
		SaleLimit *int64 `json:"sale_limit"`
	}
	_ = json.Unmarshal(listing.Product.Marketplace, &extra)
	if in.Title == "" {
		in.Title = listing.Product.Name
	}
	if in.Description == "" {
		in.Description = listing.Product.Description
	}
	if in.ImageURLs == nil {
		images := make([]string, 0, 2)
		for _, value := range []string{listing.Product.Logo, listing.Product.Image} {
			if strings.TrimSpace(value) != "" {
				images = append(images, value)
			}
		}
		in.ImageURLs = images
	}
	if in.Contact == "" {
		in.Contact = listing.Product.SupportEmail
	}
	if in.Links == nil && listing.RedemptionURL != "" {
		in.Links = []LegacyLink{{Title: "Extore", URL: listing.RedemptionURL}}
	}
	visibility := "public"
	in.Visibility = &visibility
	in.PaymentMethods = []string{"balance"}
	in.PickupLoginRequired = true
	in.DeliveryStrategy = "sequential"
	in.Variants = nil
	if err := validateLegacyInput(in); err != nil {
		return nil, err
	}
	if in.CategoryID != nil && *in.CategoryID != "" {
		if _, err := s.readCategory(ctx, *in.CategoryID, true); err != nil {
			return nil, err
		}
	}
	variants := make([]extoreVariantIn, 0, len(listing.Variants))
	seen := map[string]bool{}
	names := map[string]bool{}
	enabled := false
	for _, variant := range listing.Variants {
		price := int64(0)
		if variant.Marketplace.PriceQuota != nil {
			price = *variant.Marketplace.PriceQuota
		} else if variant.Price != nil && variant.Currency == "USD" {
			value, err := extoreUSDCredits(*variant.Price)
			if err != nil {
				return nil, ErrInvalid
			}
			price = value
		}
		if price <= 0 {
			return nil, errors.New("Configure the selling price for every variant in Extore before syncing")
		}
		if price < 500000 || price > legacySafeInteger {
			return nil, ErrInvalid
		}
		template := variant.Marketplace.Template
		if template == "" {
			template = "card-key"
		}
		if err := extoreDeliveryTemplate(template, variant.Marketplace.FixedContent); err != nil {
			return nil, err
		}
		if variant.ID == "" || seen[variant.ID] || names[strings.ToLower(variant.Name)] || !Text(variant.Name, 200) {
			return nil, ErrInvalid
		}
		seen[variant.ID] = true
		names[strings.ToLower(variant.Name)] = true
		enabled = enabled || variant.Enabled
		variants = append(variants, extoreVariantIn{
			SourceID: variant.ID, Name: variant.Name, Price: price, Template: template,
			Enabled: variant.Enabled, Fixed: variant.Marketplace.FixedContent,
		})
	}
	if !enabled || len(variants) > extoreMaxVariants {
		return nil, ErrInvalid
	}
	revision := listing.SyncRevision
	if revision == "" {
		revision = listing.Revision
	}
	if len(revision) != 64 {
		return nil, mapExtoreErr(extore.ErrProtocol)
	}
	summary := listing.CardSummary
	if len(summary) == 0 {
		summary = json.RawMessage(`{}`)
	}
	if !json.Valid(summary) {
		return nil, ErrInvalid
	}
	return s.persistExtoreProduct(ctx, userID, connectionID, listing.ID, revision, in, variants, extra.SaleLimit, summary)
}

func (s *Service) persistExtoreProduct(ctx context.Context, userID int64, connectionID, sourceID, revision string, in LegacyProductInput, variants []extoreVariantIn, saleLimit *int64, summary json.RawMessage) (*extoreProductRow, error) {
	if saleLimit != nil && (*saleLimit < 0 || *saleLimit > legacySafeInteger) {
		return nil, ErrInvalid
	}
	actor := Account{Kind: "personal", ID: userID}
	legacy, err := s.legacyShop(ctx, actor)
	if err != nil {
		return nil, err
	}
	extoreShop := extoreSellerShop(userID)
	var connection extoreConnectionRow
	err = s.repo.Within(ctx, extoreShop, func(tx Tx) error {
		if err := tx.Get(extoreConnectionKind, connectionID, &connection); err != nil {
			return err
		}
		if connection.RefreshPending || connection.ExpiresAt <= time.Now().UTC().Unix() {
			return ErrExtoreReauthorize
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	key := ID(extorePurpose, "product", userID, connection.Origin, connection.ShopID, sourceID)
	var source extoreProductRow
	now := time.Now().UTC().Unix()
	err = s.repo.Within(ctx, legacy.ID, func(tx Tx) error {
		switch err := tx.Get(extoreProductKind, key, &source); {
		case err == nil:
		case errors.Is(err, ErrNotFound):
			source = extoreProductRow{ID: key, ConnectionID: connectionID, SourceID: sourceID, VariantIDs: map[string]string{}}
		default:
			return err
		}
		if source.VariantIDs == nil {
			source.VariantIDs = map[string]string{}
		}
		if source.ProductID != "" && source.Revision == revision {
			source.CardSummary = summary
			source.LastSyncedAt = now
			source.ConnectionID = connectionID
			return tx.Put(extoreProductKind, key, source)
		}
		var p Product
		first := source.ProductID == ""
		if first {
			id, err := legacyRandomID()
			if err != nil {
				return err
			}
			p.ID = id
		} else {
			if err := tx.Get("products", source.ProductID, &p); err != nil {
				return err
			}
			if p.Legacy == nil || p.Legacy.Status == "deleted" {
				return ErrNotFound
			}
			if p.Legacy.Status == "pending" {
				return ErrConflict
			}
		}
		defaultIndex := 0
		if p.Legacy != nil && p.Legacy.DefaultVariantID != "" {
			for i, v := range variants {
				if source.VariantIDs[v.SourceID] == p.Legacy.DefaultVariantID {
					defaultIndex = i
					break
				}
			}
		}
		m := &LegacyProductMetadata{
			Description: in.Description, ImageURLs: in.ImageURLs, Contact: in.Contact, Links: in.Links,
			Visibility: "public", PurchaseLoginRequired: true, MaxQuantityPerOrder: in.MaxQuantityPerOrder,
			MaxQuantityPerBuyer: in.MaxQuantityPerBuyer, Template: variants[defaultIndex].Template,
			DeliveryStrategy: "sequential", PaymentMethods: []string{"balance"}, PickupLoginRequired: true,
			PickupCodeRequired: in.PickupCodeRequired, Status: "draft", CreatedAt: now, UpdatedAt: now,
			SaleLimit: saleLimit,
		}
		if m.ImageURLs == nil {
			m.ImageURLs = []string{}
		}
		if m.Links == nil {
			m.Links = []LegacyLink{}
		}
		if in.CategoryID != nil {
			m.CategoryID = *in.CategoryID
		}
		if in.TestMode != nil {
			m.TestMode = *in.TestMode
		}
		if !first {
			m.CreatedAt = p.Legacy.CreatedAt
			if in.CategoryID == nil {
				m.CategoryID = p.Legacy.CategoryID
			}
			if saleLimit == nil {
				m.SaleLimit = p.Legacy.SaleLimit
			}
			prior := p.Legacy.Status
			if prior == "paused" || prior == "off_shelf" {
				m.Status = prior
			}
			m.DefaultVariantID = p.Legacy.DefaultVariantID
		}
		p.Title = in.Title
		p.Published = false
		p.Legacy = m
		for i, v := range variants {
			fixed := ""
			if v.Fixed != nil {
				fixed = *v.Fixed
			}
			id := source.VariantIDs[v.SourceID]
			var existing Variant
			if id != "" {
				switch err := tx.Get("variants", id, &existing); {
				case err == nil:
				case errors.Is(err, ErrNotFound):
					id = ""
				default:
					return err
				}
			}
			if id == "" {
				next, err := legacyRandomID()
				if err != nil {
					return err
				}
				id = next
				source.VariantIDs[v.SourceID] = id
				existing = Variant{ID: id, ProductID: p.ID, Quota: -1, Legacy: &LegacyVariantMetadata{CreatedAt: now}}
			}
			if existing.Legacy == nil {
				existing.Legacy = &LegacyVariantMetadata{CreatedAt: now}
			}
			existing.ProductID = p.ID
			existing.Title = v.Name
			existing.Price = Price{Amount: v.Price, Unit: "credits"}
			existing.Quota = -1
			existing.Legacy.Template = v.Template
			existing.Legacy.Enabled = v.Enabled
			existing.Legacy.FixedContent = fixed
			existing.Legacy.UpdatedAt = now
			if err := tx.Put("variants", id, existing); err != nil {
				return err
			}
			if i == defaultIndex {
				m.DefaultVariantID = id
				m.Template = v.Template
			}
		}
		active := map[string]bool{}
		for _, v := range variants {
			active[v.SourceID] = true
		}
		for remoteID, localID := range source.VariantIDs {
			if active[remoteID] {
				continue
			}
			var retired Variant
			if err := tx.Get("variants", localID, &retired); err != nil {
				if errors.Is(err, ErrNotFound) {
					continue
				}
				return err
			}
			if retired.ProductID != p.ID || retired.Legacy == nil {
				continue
			}
			retired.Legacy.Enabled = false
			retired.Legacy.UpdatedAt = now
			if err := tx.Put("variants", localID, retired); err != nil {
				return err
			}
		}
		vs, err := variantsFor(tx, p.ID)
		if err != nil {
			return err
		}
		if err := snapshotLegacy(&p, actor, vs); err != nil {
			return err
		}
		if err := tx.Put("products", p.ID, p); err != nil {
			return err
		}
		source.ConnectionID = connectionID
		source.SourceID = sourceID
		source.ProductID = p.ID
		source.Revision = revision
		source.CardSummary = summary
		source.LastSyncedAt = now
		return tx.Put(extoreProductKind, key, source)
	})
	if err != nil {
		return nil, err
	}
	return &source, nil
}

func (s *Service) requireLocalProductSource(ctx context.Context, actor Account, productID string) error {
	if !s.ExtoreReady() || productID == "" {
		return nil
	}
	legacy, err := s.legacyShop(ctx, actor)
	if err != nil {
		return err
	}
	rows, err := s.listKind(ctx, legacy.ID, extoreProductKind)
	if err != nil {
		return err
	}
	for _, raw := range rows {
		var row extoreProductRow
		if json.Unmarshal(raw, &row) != nil {
			return ErrUnavailable
		}
		if row.ProductID == productID {
			return ErrExtoreSourceManaged
		}
	}
	return nil
}
