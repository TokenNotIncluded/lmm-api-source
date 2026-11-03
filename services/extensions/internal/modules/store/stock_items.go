// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"bytes"
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"math"
	"net/url"
	"sort"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

// StockItem is one finite delivery unit (activation key, account, link...).
// Its content is only ever stored as AES-256-GCM ciphertext under the store's
// own stock key, bound to the shop and item IDs.
type StockItem struct {
	ID         string `json:"id"`
	ProductID  string `json:"product_id"`
	VariantID  string `json:"variant_id"`
	Position   int64  `json:"position"`
	State      string `json:"state"` // available, reserved, delivered, refunded, removed
	Ciphertext string `json:"ciphertext"`
	OrderID    string `json:"order_id,omitempty"`
	CreatedAt  int64  `json:"created_at"`
	UpdatedAt  int64  `json:"updated_at"`
}

const (
	stockAvailable = "available"
	stockReserved  = "reserved"
	stockDelivered = "delivered"
	stockRefunded  = "refunded"
	stockRemoved   = "removed"
	maxStockImport = 10000
	maxStockItem   = 32768
)

type stockCipher struct{ aead cipher.AEAD }

// WithStockKey enables per-unit stock. The 32-byte key comes from a dedicated
// private file and is never shared with Core credentials or other modules.
func (s *Service) WithStockKey(key []byte) error {
	if len(key) != 32 {
		return ErrInvalid
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return err
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return err
	}
	s.stock = &stockCipher{aead}
	return nil
}

// StockReady reports whether item contents can be sealed and opened.
func (s *Service) StockReady() bool { return s.stock != nil }

func stockAAD(shop, id string) []byte { return []byte("lmm-store-stock-v1\x00" + shop + "\x00" + id) }

func (c *stockCipher) seal(shop, id, plain string) (string, error) {
	nonce := make([]byte, c.aead.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return "", err
	}
	return "v1:" + base64.RawURLEncoding.EncodeToString(c.aead.Seal(nonce, nonce, []byte(plain), stockAAD(shop, id))), nil
}
func (c *stockCipher) open(shop, id, value string) (string, error) {
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(value, "v1:"))
	if !strings.HasPrefix(value, "v1:") || err != nil || len(raw) < c.aead.NonceSize() {
		return "", ErrUnavailable
	}
	plain, err := c.aead.Open(nil, raw[:c.aead.NonceSize()], raw[c.aead.NonceSize():], stockAAD(shop, id))
	if err != nil {
		return "", ErrUnavailable
	}
	return string(plain), nil
}

// Every template except fixed-content delivers finite per-unit stock items.
func stockTemplate(template string) bool {
	switch template {
	case "card-key", "text", "custom-text", "redemption-code", "license-key", "download-link", "account-details":
		return true
	}
	return false
}
func stockOrder(o Order) bool {
	return o.Legacy != nil && stockTemplate(o.Legacy.DeliveryTemplate) && len(o.StockItems) > 0
}

// The delivery reference covers the exact item IDs without exposing contents.
func stockDeliverySummary(ids []string) string {
	sum := sha256.Sum256([]byte(strings.Join(ids, "\n")))
	return "stock-items:v1:" + hex.EncodeToString(sum[:])
}

type deliveryField struct {
	required bool
	max      int
	url      bool
}

func structuredDeliveryFields(template string) map[string]deliveryField {
	instructions := deliveryField{max: 16384}
	secret := deliveryField{required: true, max: 4096}
	optionalURL := deliveryField{max: 2048, url: true}
	switch template {
	case "redemption-code":
		return map[string]deliveryField{"code": secret, "redeem_url": optionalURL, "instructions": instructions}
	case "license-key":
		return map[string]deliveryField{"license_key": secret, "product": {max: 1000}, "instructions": instructions}
	case "download-link":
		return map[string]deliveryField{"url": {required: true, max: 2048, url: true}, "access_code": {max: 4096}, "instructions": instructions}
	case "account-details":
		return map[string]deliveryField{"username": {required: true, max: 1000}, "password": secret, "url": optionalURL, "instructions": instructions}
	}
	return nil
}
func deliveryURL(value string) bool {
	if strings.TrimSpace(value) != value || strings.IndexFunc(value, unicode.IsControl) >= 0 {
		return false
	}
	u, err := url.Parse(value)
	return err == nil && (u.Scheme == "http" || u.Scheme == "https") && u.Hostname() != "" && u.User == nil && u.Opaque == ""
}

// validateStockItem preserves the legacy rule: each element is one unit.
// Only a matching lmm_store_delivery v1 envelope is validated as structured.
func validateStockItem(template, item string) error {
	if strings.TrimSpace(item) == "" || len(item) > maxStockItem || !utf8.ValidString(item) || strings.ContainsRune(item, 0) {
		return ErrInvalid
	}
	var envelope map[string]json.RawMessage
	if json.Unmarshal([]byte(item), &envelope) != nil {
		return nil
	}
	var marker any
	if json.Unmarshal(envelope["lmm_store_delivery"], &marker) != nil {
		return nil
	}
	if version, numeric := marker.(float64); !numeric || version != 1 {
		return nil
	}
	var kind string
	if json.Unmarshal(envelope["template"], &kind) != nil || kind != template {
		return nil
	}
	fields := structuredDeliveryFields(kind)
	if fields == nil {
		return nil
	}
	var values map[string]any
	if len(envelope) != 3 || json.Unmarshal(envelope["fields"], &values) != nil || values == nil {
		return ErrInvalid
	}
	for key, raw := range values {
		field, ok := fields[key]
		value, text := raw.(string)
		if !ok || !text || len(value) > field.max || value != "" && field.url && !deliveryURL(value) {
			return ErrInvalid
		}
	}
	for key, field := range fields {
		if value, _ := values[key].(string); field.required && strings.TrimSpace(value) == "" {
			return ErrInvalid
		}
	}
	return nil
}

// stockQueryTx is an optional indexed lookup. Other transactions fall back to
// a bounded scan of the shop's stock rows.
type stockQueryTx interface {
	StockItems(variantID, state string, offset, limit int, random bool) ([]StockItem, error)
}
type bulkPutTx interface {
	PutAll(kind string, ids []string, values []any) error
}

func queryStock(tx Tx, variantID, state string, offset, limit int, random bool) ([]StockItem, error) {
	if q, ok := tx.(stockQueryTx); ok {
		return q.StockItems(variantID, state, offset, limit, random)
	}
	var all []StockItem
	after := ""
	for page := 0; ; page++ {
		if page > 1000 {
			return nil, ErrUnavailable
		}
		rows, err := tx.List("stock_items", after, 100)
		if err != nil {
			return nil, err
		}
		for _, raw := range rows {
			var item StockItem
			if json.Unmarshal(raw, &item) != nil {
				return nil, ErrUnavailable
			}
			after = item.ID
			if item.VariantID == variantID && item.State != stockRemoved && (state == "" || item.State == state) {
				all = append(all, item)
			}
		}
		if len(rows) < 100 {
			break
		}
	}
	sort.Slice(all, func(i, j int) bool {
		if all[i].Position != all[j].Position {
			return all[i].Position < all[j].Position
		}
		return all[i].ID < all[j].ID
	})
	if random {
		for i := len(all) - 1; i > 0; i-- {
			var b [8]byte
			if _, err := rand.Read(b[:]); err != nil {
				return nil, err
			}
			j := int(uint64(b[0])|uint64(b[1])<<8|uint64(b[2])<<16|uint64(b[3])<<24|uint64(b[4])<<32) % (i + 1)
			all[i], all[j] = all[j], all[i]
		}
	}
	if offset >= len(all) {
		return []StockItem{}, nil
	}
	all = all[offset:]
	if len(all) > limit {
		all = all[:limit]
	}
	return all, nil
}

// allocateStock reserves exactly n available units for an order under the shop
// lock. The caller has already reserved the same quantity on the variant.
func allocateStock(tx Tx, v Variant, orderID string, n int64, random bool) ([]string, error) {
	if n <= 0 || n > maxStockImport {
		return nil, ErrInvalid
	}
	items, err := queryStock(tx, v.ID, stockAvailable, 0, int(n), random)
	if err != nil {
		return nil, err
	}
	if int64(len(items)) != n {
		return nil, ErrSoldOut
	}
	now := time.Now().Unix()
	ids := make([]string, 0, n)
	for _, item := range items {
		item.State, item.OrderID, item.UpdatedAt = stockReserved, orderID, now
		if err := tx.Put("stock_items", item.ID, item); err != nil {
			return nil, err
		}
		ids = append(ids, item.ID)
	}
	return ids, nil
}

// moveStock transitions an order's items. Only the listed source state moves;
// an item in any other state is a conflict, never silently skipped.
func moveStock(tx Tx, o Order, from, to string) error {
	now := time.Now().Unix()
	for _, id := range o.StockItems {
		var item StockItem
		if err := tx.Get("stock_items", id, &item); err != nil {
			return err
		}
		if item.OrderID != o.ID || item.VariantID != o.VariantID {
			return ErrConflict
		}
		if item.State == to {
			continue
		}
		if item.State != from {
			return ErrConflict
		}
		item.State, item.UpdatedAt = to, now
		if to == stockAvailable {
			item.OrderID = ""
		}
		if err := tx.Put("stock_items", item.ID, item); err != nil {
			return err
		}
	}
	return nil
}

func (s *Service) openOrderStock(tx Tx, shopID string, o Order) ([]string, []string, []int64, error) {
	if s.stock == nil {
		return nil, nil, nil, ErrUnavailable
	}
	items, ids, positions := []string{}, []string{}, []int64{}
	for i, id := range o.StockItems {
		var item StockItem
		if err := tx.Get("stock_items", id, &item); err != nil {
			return nil, nil, nil, err
		}
		if item.OrderID != o.ID || item.State != stockDelivered {
			continue
		}
		plain, err := s.stock.open(shopID, item.ID, item.Ciphertext)
		if err != nil {
			return nil, nil, nil, err
		}
		items, ids, positions = append(items, plain), append(ids, item.ID), append(positions, int64(i+1))
	}
	return items, ids, positions, nil
}

func stockView(item StockItem) map[string]any {
	return map[string]any{"id": item.ID, "product_id": item.ProductID, "variant_id": item.VariantID, "state": item.State, "position": item.Position, "created_at": item.CreatedAt}
}

// legacyStock implements the seller inventory routes for one product variant.
func (s *Service) legacyStock(ctx context.Context, actor Account, r LegacyRequest) (any, error) {
	row, err := s.LocateProduct(ctx, r.ID)
	if err != nil {
		return nil, err
	}
	if row.Owner != actor {
		return nil, ErrForbidden
	}
	if row.Product.Legacy == nil || row.Product.Legacy.Status == "deleted" {
		return nil, ErrNotFound
	}
	variantID := r.ChildID
	if variantID == "" {
		variantID = row.Product.Legacy.DefaultVariantID
	}
	var items []string
	if r.Action == "stock_add" {
		if s.stock == nil {
			return nil, ErrUnavailable
		}
		var in struct {
			Items []string `json:"items"`
		}
		d := json.NewDecoder(bytes.NewReader(r.Body))
		d.DisallowUnknownFields()
		if len(r.Body) == 0 || len(r.Body) > 4<<20 || d.Decode(&in) != nil || d.Decode(new(any)) != io.EOF || len(in.Items) == 0 || len(in.Items) > maxStockImport {
			return nil, ErrInvalid
		}
		items = in.Items
	} else if r.Action == "stock_list" && (r.Offset < 0 || r.Offset > 100000 || !Page(r.Limit)) {
		return nil, ErrInvalid
	}
	var out any
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var v Variant
		if err := tx.Get("variants", variantID, &v); err != nil {
			return err
		}
		if v.ProductID != row.Product.ID || v.Legacy == nil {
			return ErrNotFound
		}
		if !stockTemplate(v.Legacy.Template) {
			return ErrInvalid
		}
		switch r.Action {
		case "stock_list":
			rows, err := queryStock(tx, v.ID, "", r.Offset, r.Limit, false)
			if err != nil {
				return err
			}
			views := make([]map[string]any, 0, len(rows))
			for _, item := range rows {
				views = append(views, stockView(item))
			}
			out = legacyPage(views, len(views), r.Offset, r.Limit)
			return nil
		case "stock_add":
			if !v.TrackStock || v.StockSequence > math.MaxInt64-int64(len(items)) || v.Stock > math.MaxInt64-int64(len(items)) {
				return ErrConflict
			}
			now := time.Now().Unix()
			ids := make([]string, 0, len(items))
			values := make([]any, 0, len(items))
			for _, plain := range items {
				if err := validateStockItem(v.Legacy.Template, plain); err != nil {
					return err
				}
				id, err := legacyRandomID()
				if err != nil {
					return err
				}
				sealed, err := s.stock.seal(row.ShopID, id, plain)
				if err != nil {
					return err
				}
				v.StockSequence++
				ids = append(ids, id)
				values = append(values, StockItem{ID: id, ProductID: v.ProductID, VariantID: v.ID, Position: v.StockSequence, State: stockAvailable, Ciphertext: sealed, CreatedAt: now, UpdatedAt: now})
			}
			if bulk, ok := tx.(bulkPutTx); ok {
				if err := bulk.PutAll("stock_items", ids, values); err != nil {
					return err
				}
			} else {
				for i := range ids {
					if err := tx.Put("stock_items", ids[i], values[i]); err != nil {
						return err
					}
				}
			}
			v.Stock += int64(len(items))
			out = map[string]any{"added": len(items)}
		case "stock_remove":
			var item StockItem
			if err := tx.Get("stock_items", r.StockID, &item); err != nil {
				return err
			}
			if item.VariantID != v.ID || item.ProductID != v.ProductID || item.State == stockRemoved {
				return ErrNotFound
			}
			if item.State != stockAvailable || v.Stock-v.Reserved < 1 {
				return ErrConflict
			}
			// The runtime role cannot delete rows; a removed unit keeps no content.
			item.State, item.Ciphertext, item.UpdatedAt = stockRemoved, "", time.Now().Unix()
			if err := tx.Put("stock_items", item.ID, item); err != nil {
				return err
			}
			v.Stock--
		default:
			return ErrInvalid
		}
		if err := tx.Put("variants", v.ID, v); err != nil {
			return err
		}
		var p Product
		if err := tx.Get("products", row.Product.ID, &p); err != nil {
			return err
		}
		vs, err := variantsFor(tx, p.ID)
		if err != nil {
			return err
		}
		if err := snapshotLegacy(&p, row.Owner, vs); err != nil {
			return err
		}
		return tx.Put("products", p.ID, p)
	})
	if errors.Is(err, ErrNotFound) && r.Action == "stock_remove" {
		return nil, ErrNotFound
	}
	return out, err
}
