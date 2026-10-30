// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/json"
	"math"
)

type buyerQuantityReader interface {
	BuyerProductQuantity(string, Account) (int64, error)
}

// Called while holding the shop transaction lock. Pending orders count as well
// as paid orders; only a confirmed cancellation or full refund releases quota.
func buyerProductQuantity(tx Tx, product string, buyer Account) (int64, error) {
	if reader, ok := tx.(buyerQuantityReader); ok {
		return reader.BuyerProductQuantity(product, buyer)
	}
	var total int64
	after := ""
	for {
		rows, err := tx.List("orders", after, 100)
		if err != nil {
			return 0, err
		}
		for _, raw := range rows {
			var order Order
			if json.Unmarshal(raw, &order) != nil || order.ID <= after {
				return 0, ErrUnavailable
			}
			after = order.ID
			if order.Buyer != buyer || order.State == Cancelled || order.State == Refunded {
				continue
			}
			var variant Variant
			if err := tx.Get("variants", order.VariantID, &variant); err != nil {
				return 0, err
			}
			if variant.ProductID != product {
				continue
			}
			if order.Quantity < 1 || total > math.MaxInt64-order.Quantity {
				return 0, ErrUnavailable
			}
			total += order.Quantity
		}
		if len(rows) < 100 {
			return total, nil
		}
	}
}

func (t *postgresTx) BuyerProductQuantity(product string, buyer Account) (int64, error) {
	var total int64
	err := t.tx.QueryRowContext(t.ctx, `SELECT COALESCE(SUM((o.body->>'quantity')::bigint),0)
FROM lmm_store.orders o JOIN lmm_store.variants v ON v.shop_id=o.shop_id AND v.id=o.body->>'variant_id'
WHERE o.shop_id=$1 AND v.body->>'product_id'=$2 AND o.body->'buyer'->>'kind'=$3
AND (o.body->'buyer'->>'id')::bigint=$4 AND o.body->>'state' NOT IN ('cancelled','refunded')`, t.shop, product, buyer.Kind, buyer.ID).Scan(&total)
	return total, err
}

func (s *Service) ProductForBuyer(ctx context.Context, token string, user int64, id string) (map[string]any, error) {
	view, err := s.PublicProduct(ctx, id)
	if err != nil || user == 0 {
		return view, err
	}
	buyer := Account{Kind: "personal", ID: user}
	if _, err := s.check(ctx, token, buyer, Read); err != nil {
		return nil, err
	}
	row, err := s.LocateProduct(ctx, id)
	if err != nil {
		return nil, err
	}
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var product Product
		if err := tx.Get("products", id, &product); err != nil {
			return err
		}
		if product.Legacy == nil || product.Legacy.MaxQuantityPerBuyer == nil {
			return nil
		}
		used, err := buyerProductQuantity(tx, id, buyer)
		if err != nil {
			return err
		}
		view["buyer_purchase_remaining"] = max(int64(0), *product.Legacy.MaxQuantityPerBuyer-used)
		return nil
	})
	return view, err
}
