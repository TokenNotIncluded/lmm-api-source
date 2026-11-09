package store

import (
	"context"
	"errors"
)

type cartInput struct {
	ProductID string `json:"product_id"`
	VariantID string `json:"variant_id"`
	Quantity  int64  `json:"quantity"`
}

type collectionReference struct {
	ID        string `json:"id"`
	ProductID string `json:"product_id"`
	VariantID string `json:"variant_id"`
	Quantity  int64  `json:"quantity"`
	CreatedAt int64  `json:"created_at"`
	UpdatedAt int64  `json:"updated_at"`
}

type collectionRepository interface {
	readCollection(context.Context, int64, bool, int, int) ([]collectionReference, error)
	putCollection(context.Context, int64, bool, collectionReference) (collectionReference, error)
	deleteCollection(context.Context, int64, bool, string) (int64, error)
	cleanupCollections(context.Context, int64) (int64, int64, error)
}

func (s *Service) collections(ctx context.Context, actor Account, r LegacyRequest) (any, error) {
	repo, ok := s.repo.(collectionRepository)
	if !ok {
		return nil, ErrUnavailable
	}
	if r.Action != "save_cart" && r.Action != "save_favorite" && len(r.Body) != 0 {
		var empty struct{}
		if legacyDecode(r.Body, &empty) != nil {
			return nil, ErrInvalid
		}
	}
	switch r.Action {
	case "cart", "favorites":
		if r.Offset < 0 || r.Offset > 100000 || !Page(r.Limit) {
			return nil, ErrInvalid
		}
		cart := r.Action == "cart"
		rows, err := repo.readCollection(ctx, actor.ID, cart, r.Offset, r.Limit)
		if err != nil {
			return nil, err
		}
		items := make([]map[string]any, 0, len(rows))
		for _, row := range rows {
			product, reason, err := s.collectionProduct(ctx, actor, row, cart)
			if err != nil {
				return nil, err
			}
			var unavailable any
			if reason != "" {
				unavailable = reason
			}
			item := map[string]any{"product_id": row.ProductID, "created_at": row.CreatedAt, "valid": reason == "", "unavailable_reason": unavailable, "product": product}
			if cart {
				item["id"], item["variant_id"], item["quantity"], item["updated_at"] = row.ID, row.VariantID, row.Quantity, row.UpdatedAt
			}
			items = append(items, item)
		}
		return legacyPage(items, len(items), r.Offset, r.Limit), nil
	case "save_cart", "save_favorite":
		var ref collectionReference
		cart := r.Action == "save_cart"
		if cart {
			var in cartInput
			if legacyDecode(r.Body, &in) != nil || !Text(in.ProductID, 128) || !Text(in.VariantID, 128) || in.Quantity < 1 || in.Quantity > 1000 {
				return nil, ErrInvalid
			}
			ref.ProductID, ref.VariantID, ref.Quantity = in.ProductID, in.VariantID, in.Quantity
			ref.ID = ID("store-cart-v1", actor.ID, in.ProductID, in.VariantID)
		} else {
			var in struct {
				ProductID string `json:"product_id"`
			}
			if legacyDecode(r.Body, &in) != nil || !Text(in.ProductID, 128) {
				return nil, ErrInvalid
			}
			ref.ProductID = in.ProductID
		}
		_, reason, err := s.collectionProduct(ctx, actor, ref, cart)
		if err != nil {
			return nil, err
		}
		switch reason {
		case "not_visible":
			return nil, ErrNotFound
		case "insufficient_stock":
			return nil, ErrSoldOut
		case "purchase_limit":
			return nil, ErrPurchaseLimit
		case "unavailable":
			return nil, ErrConflict
		}
		stored, err := repo.putCollection(ctx, actor.ID, cart, ref)
		if err != nil || !cart {
			return nil, err
		}
		return stored, nil
	case "delete_cart", "delete_favorite", "clear_cart", "clear_favorites":
		id := r.ID
		if r.Action == "clear_cart" || r.Action == "clear_favorites" {
			id = ""
		} else if !Text(id, 128) {
			return nil, ErrInvalid
		}
		_, err := repo.deleteCollection(ctx, actor.ID, r.Action == "delete_cart" || r.Action == "clear_cart", id)
		return nil, err
	case "cleanup_collections":
		cart, favorites, err := repo.cleanupCollections(ctx, actor.ID)
		return map[string]any{"cart_deleted": cart, "favorites_deleted": favorites}, err
	default:
		return nil, ErrInvalid
	}
}

// Pausing a product suspends checkout but keeps an account's saved reference.
// A privacy change immediately removes its content from collection responses.
func (s *Service) collectionProduct(ctx context.Context, actor Account, ref collectionReference, cart bool) (map[string]any, string, error) {
	row, err := s.LocateProduct(ctx, ref.ProductID)
	if errors.Is(err, ErrNotFound) {
		return nil, "not_visible", nil
	}
	if err != nil {
		return nil, "", err
	}
	retained := row
	if m := row.Product.Legacy; m != nil && m.Status == "paused" {
		copy := *m
		copy.Status = "published"
		retained.Product.Legacy = &copy
		retained.Product.Published = true
	}
	visible, err := productVisibleTo(retained, actor.ID)
	if err != nil {
		return nil, "", err
	}
	if !visible {
		return nil, "not_visible", nil
	}
	view, err := s.publicLegacyViewFor(row, false, actor.ID)
	if err != nil {
		return nil, "", err
	}
	view["review_note"], view["reviewed_by"] = "", 0
	if !cart {
		return view, "", nil
	}
	m := row.Product.Legacy
	ready, err := productPurchaseReady(row, actor)
	if err != nil {
		return nil, "", err
	}
	if m == nil || !ready || !s.funds.Available() || row.Owner == actor && !s.selfPurchaseAvailable() {
		return view, "unavailable", nil
	}
	for _, v := range row.Variants {
		if v.ID != ref.VariantID {
			continue
		}
		if v.Legacy == nil || !v.Legacy.Enabled || v.Price.Unit != "credits" || v.Price.Amount <= 0 || ref.Quantity > legacySafeInteger/v.Price.Amount {
			return view, "unavailable", nil
		}
		if m.MaxQuantityPerOrder != nil && ref.Quantity > *m.MaxQuantityPerOrder {
			return view, "purchase_limit", nil
		}
		if m.MaxQuantityPerBuyer != nil {
			var used int64
			err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
				var err error
				used, err = buyerProductQuantity(tx, row.Product.ID, actor)
				return err
			})
			if err != nil {
				return nil, "", err
			}
			remaining := max(int64(0), *m.MaxQuantityPerBuyer-used)
			view["buyer_purchase_remaining"] = remaining
			if ref.Quantity > remaining {
				return view, "purchase_limit", nil
			}
		}
		if v.TrackStock && ref.Quantity > v.Stock-v.Reserved || v.Quota >= 0 && ref.Quantity > v.Quota-v.Claimed {
			return view, "insufficient_stock", nil
		}
		if available := view["sale_available"].(int64); available >= 0 && ref.Quantity > available {
			return view, "insufficient_stock", nil
		}
		return view, "", nil
	}
	return view, "unavailable", nil
}
