// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"net/url"
	"strings"
	"time"
)

type Promotion struct {
	ID            string   `json:"id"`
	ProductID     string   `json:"product_id"`
	SellerID      int64    `json:"seller_id"`
	Code          string   `json:"code"`
	DiscountBPS   int64    `json:"discount_bps"`
	VariantIDs    []string `json:"variant_ids"`
	ExpiresAt     *int64   `json:"expires_at"`
	MaxUses       *int64   `json:"max_uses"`
	Status        string   `json:"status"`
	UsesCount     int64    `json:"uses_count"`
	ReservedCount int64    `json:"reserved_count"`
	CreatedAt     int64    `json:"created_at"`
	UpdatedAt     int64    `json:"updated_at"`
	SharePath     string   `json:"share_path"`
}

type promotionList struct {
	Items []Promotion `json:"items"`
}

type promotionCount struct{ used, reserved int64 }

func readPromotions(tx Tx, product string) (promotionList, error) {
	var list promotionList
	err := tx.Get("commands", "@web:promotions:"+product, &list)
	if errors.Is(err, ErrNotFound) {
		err = nil
	}
	return list, err
}

func promotionUsage(tx Tx, product string) (map[string]promotionCount, error) {
	if reader, ok := tx.(interface {
		PromotionUsage(string) (map[string]promotionCount, error)
	}); ok {
		return reader.PromotionUsage(product)
	}
	counts := map[string]promotionCount{}
	after := ""
	for {
		rows, err := tx.List("orders", after, 100)
		if err != nil {
			return nil, err
		}
		for _, raw := range rows {
			var order Order
			if json.Unmarshal(raw, &order) != nil || order.ID <= after {
				return nil, ErrUnavailable
			}
			after = order.ID
			if order.Legacy == nil || order.Legacy.ProductID != product || order.Legacy.PromotionID == "" || order.State == Cancelled || order.State == Refunded {
				continue
			}
			count := counts[order.Legacy.PromotionID]
			if order.State == AwaitingPayment || order.State == PaymentPending {
				count.reserved++
			} else {
				count.used++
			}
			counts[order.Legacy.PromotionID] = count
		}
		if len(rows) < 100 {
			return counts, nil
		}
	}
}

func (t *postgresTx) PromotionUsage(product string) (map[string]promotionCount, error) {
	rows, err := t.tx.QueryContext(t.ctx, `SELECT body->'legacy'->>'promotion_id',
COUNT(*) FILTER (WHERE body->>'state' NOT IN ('awaiting_payment','payment_pending')),
COUNT(*) FILTER (WHERE body->>'state' IN ('awaiting_payment','payment_pending'))
FROM lmm_store.orders WHERE shop_id=$1 AND body->'legacy'->>'product_id'=$2
AND COALESCE(body->'legacy'->>'promotion_id','')<>'' AND body->>'state' NOT IN ('cancelled','refunded')
GROUP BY body->'legacy'->>'promotion_id'`, t.shop, product)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	counts := map[string]promotionCount{}
	for rows.Next() {
		var id string
		var count promotionCount
		if err := rows.Scan(&id, &count.used, &count.reserved); err != nil {
			return nil, err
		}
		counts[id] = count
	}
	return counts, rows.Err()
}

func promotionAvailable(p Promotion, variant string, now int64) bool {
	if p.Status != "active" || p.ExpiresAt != nil && *p.ExpiresAt <= now || p.MaxUses != nil && p.UsesCount+p.ReservedCount >= *p.MaxUses {
		return false
	}
	if len(p.VariantIDs) == 0 || variant == "" {
		return true
	}
	for _, id := range p.VariantIDs {
		if id == variant {
			return true
		}
	}
	return false
}

func promotionByCode(tx Tx, product, variant, code string) (Promotion, error) {
	list, err := readPromotions(tx, product)
	if err != nil {
		return Promotion{}, err
	}
	counts, err := promotionUsage(tx, product)
	if err != nil {
		return Promotion{}, err
	}
	for _, p := range list.Items {
		if p.Code != code {
			continue
		}
		count := counts[p.ID]
		p.UsesCount, p.ReservedCount = count.used, count.reserved
		if !promotionAvailable(p, variant, time.Now().Unix()) {
			return Promotion{}, ErrConflict
		}
		return p, nil
	}
	return Promotion{}, ErrNotFound
}

// Round the per-unit discount down once. The frozen unit price times quantity
// remains exactly equal to the order total and to Core's commerce snapshot.
func discountedUnit(amount, bps int64) (int64, error) {
	if amount < 1 || amount > legacySafeInteger || bps < 0 || bps > 10000 {
		return 0, ErrInvalid
	}
	discount := amount/10000*bps + amount%10000*bps/10000
	return amount - discount, nil
}

func (s *Service) legacyPromotions(ctx context.Context, actor Account, r LegacyRequest) (any, error) {
	row, err := s.LocateProduct(ctx, r.ID)
	if err != nil {
		return nil, err
	}
	if row.Owner != actor {
		return nil, ErrForbidden
	}
	var output any
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var product Product
		if err := tx.Get("products", r.ID, &product); err != nil {
			return err
		}
		if product.Legacy == nil || product.Legacy.Status == "deleted" {
			return ErrNotFound
		}
		list, err := readPromotions(tx, r.ID)
		if err != nil {
			return err
		}
		counts, err := promotionUsage(tx, r.ID)
		if err != nil {
			return err
		}
		for i := range list.Items {
			count := counts[list.Items[i].ID]
			list.Items[i].UsesCount, list.Items[i].ReservedCount = count.used, count.reserved
		}
		now := time.Now().Unix()
		switch r.Action {
		case "promotions":
			if r.Offset < 0 || r.Offset > 100000 || !Page(r.Limit) {
				return ErrInvalid
			}
			start, end := min(r.Offset, len(list.Items)), min(r.Offset+r.Limit, len(list.Items))
			items := append([]Promotion{}, list.Items[start:end]...)
			output = map[string]any{"items": items, "offset": r.Offset, "limit": r.Limit, "has_more": end < len(list.Items)}
			return nil
		case "save_promotion":
			var in struct {
				DiscountBPS *int64   `json:"discount_bps"`
				VariantIDs  []string `json:"variant_ids"`
				ExpiresAt   *int64   `json:"expires_at"`
				MaxUses     *int64   `json:"max_uses"`
				Status      string   `json:"status"`
			}
			if legacyDecode(r.Body, &in) != nil || in.DiscountBPS == nil || *in.DiscountBPS < 0 || *in.DiscountBPS > 10000 || len(in.VariantIDs) > 30 || in.MaxUses != nil && (*in.MaxUses < 1 || *in.MaxUses > 1000000) || in.ExpiresAt != nil && *in.ExpiresAt <= now || in.Status != "active" && in.Status != "paused" && in.Status != "revoked" {
				return ErrInvalid
			}
			seen := map[string]bool{}
			for _, id := range in.VariantIDs {
				var variant Variant
				if seen[id] || tx.Get("variants", id, &variant) != nil || variant.ProductID != product.ID {
					return ErrInvalid
				}
				seen[id] = true
			}
			position := -1
			var p Promotion
			for i, candidate := range list.Items {
				if candidate.ID == r.ChildID {
					position, p = i, candidate
				}
			}
			if r.ChildID == "" {
				if len(list.Items) >= 1000 {
					return ErrInvalid
				}
				id, err := legacyRandomID()
				if err != nil {
					return err
				}
				p = Promotion{ID: id, ProductID: r.ID, SellerID: actor.ID, Code: strings.ToUpper(strings.ReplaceAll(id, "-", "")), CreatedAt: now}
				p.SharePath = "/store/products/" + url.PathEscape(r.ID) + "?promotion=" + p.Code
			} else if position < 0 {
				return ErrNotFound
			}
			if in.VariantIDs == nil {
				in.VariantIDs = []string{}
			}
			p.DiscountBPS, p.VariantIDs, p.ExpiresAt, p.MaxUses, p.Status, p.UpdatedAt = *in.DiscountBPS, in.VariantIDs, in.ExpiresAt, in.MaxUses, in.Status, now
			if position < 0 {
				list.Items = append(list.Items, p)
			} else {
				list.Items[position] = p
			}
			output = p
		case "promotion_batch":
			var in struct {
				IDs    []string `json:"ids"`
				Action string   `json:"action"`
			}
			if legacyDecode(r.Body, &in) != nil || len(in.IDs) == 0 || len(in.IDs) > 100 || in.Action != "pause" && in.Action != "resume" && in.Action != "revoke" && in.Action != "delete" {
				return ErrInvalid
			}
			ids := map[string]bool{}
			for _, id := range in.IDs {
				if !categoryID(id) || ids[id] {
					return ErrInvalid
				}
				ids[id] = true
			}
			affected := 0
			next := make([]Promotion, 0, len(list.Items))
			for _, p := range list.Items {
				if !ids[p.ID] {
					next = append(next, p)
					continue
				}
				affected++
				if in.Action == "delete" {
					if p.UsesCount+p.ReservedCount > 0 {
						return ErrConflict
					}
					continue
				}
				p.Status = map[string]string{"pause": "paused", "resume": "active", "revoke": "revoked"}[in.Action]
				p.UpdatedAt = now
				next = append(next, p)
			}
			if affected != len(ids) {
				return ErrNotFound
			}
			list.Items = next
			output = map[string]any{"affected": affected}
		case "promotion_cleanup":
			var in struct {
				Limit int `json:"limit"`
			}
			if legacyDecode(r.Body, &in) != nil || !Page(in.Limit) {
				return ErrInvalid
			}
			deleted := 0
			next := make([]Promotion, 0, len(list.Items))
			for _, p := range list.Items {
				if deleted < in.Limit && p.UsesCount+p.ReservedCount == 0 && (p.Status == "revoked" || p.ExpiresAt != nil && *p.ExpiresAt <= now) {
					deleted++
					continue
				}
				next = append(next, p)
			}
			list.Items = next
			output = map[string]any{"deleted": deleted}
		default:
			return ErrInvalid
		}
		return tx.Put("commands", "@web:promotions:"+r.ID, list)
	})
	return output, err
}

func (s *Service) ResolvePromotion(ctx context.Context, product, code string) (map[string]any, error) {
	if !Text(code, 128) {
		return nil, ErrInvalid
	}
	row, err := s.LocateProduct(ctx, product)
	if err != nil {
		return nil, err
	}
	if _, err = s.publicLegacyView(row, true); err != nil {
		return nil, err
	}
	var p Promotion
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var err error
		p, err = promotionByCode(tx, product, "", code)
		return err
	})
	if err != nil {
		return nil, err
	}
	return map[string]any{"product_id": product, "code": p.Code, "discount_bps": p.DiscountBPS, "variant_ids": p.VariantIDs, "expires_at": p.ExpiresAt, "status": p.Status}, nil
}

func (s *Service) quotePromotion(ctx context.Context, actor Account, r LegacyRequest) (any, error) {
	var in struct {
		Code     string `json:"promotion_code"`
		Variant  string `json:"variant_id"`
		Quantity int64  `json:"quantity"`
	}
	if legacyDecode(r.Body, &in) != nil || !Text(in.Code, 128) || !Key(in.Variant) || in.Quantity < 1 || in.Quantity > 1000 {
		return nil, ErrInvalid
	}
	row, err := s.LocateProduct(ctx, r.ID)
	if err != nil {
		return nil, err
	}
	if _, err = s.publicLegacyView(row, true); err != nil {
		return nil, err
	}
	var result map[string]any
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var product Product
		var variant Variant
		if err := tx.Get("products", r.ID, &product); err != nil {
			return err
		}
		if err := tx.Get("variants", in.Variant, &variant); err != nil {
			return err
		}
		if product.Legacy == nil || !product.Published || !product.Legacy.Approved || product.Legacy.Status != "published" || product.Legacy.TestMode || variant.ProductID != product.ID || variant.Legacy == nil || !variant.Legacy.Enabled || variant.Price.Unit != "credits" {
			return ErrConflict
		}
		p, err := promotionByCode(tx, r.ID, in.Variant, in.Code)
		if err != nil {
			return err
		}
		unit, err := discountedUnit(variant.Price.Amount, p.DiscountBPS)
		if err != nil || variant.Price.Amount > legacySafeInteger/in.Quantity {
			return ErrInvalid
		}
		capacity := int64(1000)
		if variant.TrackStock {
			capacity = min(capacity, variant.Stock-variant.Reserved)
		}
		if variant.Quota >= 0 {
			capacity = min(capacity, variant.Quota-variant.Claimed)
		}
		m := product.Legacy
		if m.MaxQuantityPerOrder != nil {
			capacity = min(capacity, *m.MaxQuantityPerOrder)
		}
		if m.MaxQuantityPerBuyer != nil {
			used, err := buyerProductQuantity(tx, product.ID, actor)
			if err != nil {
				return err
			}
			capacity = min(capacity, *m.MaxQuantityPerBuyer-used)
		}
		if m.SaleLimit != nil {
			variants, err := variantsFor(tx, product.ID)
			if err != nil {
				return err
			}
			var used int64
			for _, v := range variants {
				if v.Claimed < 0 || used > math.MaxInt64-v.Claimed {
					return ErrUnavailable
				}
				used += v.Claimed
			}
			capacity = min(capacity, *m.SaleLimit-used)
		}
		capacity = max(int64(0), capacity)
		allowed := row.Owner != actor && in.Quantity <= capacity && (unit == 0 || s.funds.Available())
		methods := []string{}
		if allowed {
			if unit == 0 {
				methods = append(methods, "free")
			} else {
				methods = append(methods, "balance")
			}
		}
		total, original := unit*in.Quantity, variant.Price.Amount*in.Quantity
		result = map[string]any{"product_id": r.ID, "variant_id": in.Variant, "quantity": in.Quantity, "promotion_code": p.Code, "discount_bps": p.DiscountBPS, "original_price_quota": original, "discount_quota": original - total, "price_quota": total, "free": total == 0, "checkout_allowed": allowed, "max_quantity": capacity, "payment_methods": methods}
		return nil
	})
	return result, err
}
