package store

import (
	"context"
	"errors"
	"time"
)

type productLike struct {
	ProductID string `json:"product_id"`
	UserID    int64  `json:"user_id"`
	Liked     bool   `json:"liked"`
	UpdatedAt int64  `json:"updated_at"`
}

type productLikeStats interface {
	ProductLikeStats(productID string, userID int64) (int64, bool, error)
}

func productLikeID(productID string, userID int64) string {
	return ID("store-product-like-v1", productID, userID)
}

func productLikesView(count int64, liked bool) map[string]any {
	return map[string]any{"supported": true, "count": count, "liked": liked}
}

// ProductLikes returns the public like count and whether the optional viewer
// currently likes the published product.
func (s *Service) ProductLikes(ctx context.Context, productID string, userID int64) (map[string]any, error) {
	if userID < 0 || !Text(productID, 128) {
		return nil, ErrInvalid
	}
	row, err := s.LocateProduct(ctx, productID)
	if err != nil {
		return nil, err
	}
	if _, err = s.publicLegacyViewFor(row, true, userID); err != nil {
		return nil, err
	}
	var count int64
	var liked bool
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		stats, ok := tx.(productLikeStats)
		if !ok {
			return ErrUnavailable
		}
		var e error
		count, liked, e = stats.ProductLikeStats(productID, userID)
		return e
	})
	if err != nil {
		return nil, err
	}
	return productLikesView(count, liked), nil
}

// SetProductLike records or clears the caller's like. Idempotent for the same
// liked value; count always comes from currently liked rows.
func (s *Service) SetProductLike(ctx context.Context, token string, userID int64, productID string, liked bool) (map[string]any, error) {
	if userID <= 0 {
		return nil, ErrUnauthorized
	}
	if !Text(productID, 128) {
		return nil, ErrInvalid
	}
	if _, err := s.check(ctx, token, Account{Kind: "personal", ID: userID}, Read); err != nil {
		return nil, err
	}
	row, err := s.LocateProduct(ctx, productID)
	if err != nil {
		return nil, err
	}
	if _, err = s.publicLegacyViewFor(row, true, userID); err != nil {
		return nil, err
	}
	now := time.Now().UTC().Unix()
	id := productLikeID(productID, userID)
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		item := productLike{ProductID: productID, UserID: userID, Liked: liked, UpdatedAt: now}
		var existing productLike
		switch err := tx.Get("product_likes", id, &existing); {
		case err == nil:
			if existing.ProductID != productID || existing.UserID != userID {
				return ErrConflict
			}
			if existing.Liked == liked {
				return nil
			}
			item.UpdatedAt = now
			return tx.Put("product_likes", id, item)
		case errors.Is(err, ErrNotFound):
			if !liked {
				return nil
			}
			return tx.Put("product_likes", id, item)
		default:
			return err
		}
	})
	if err != nil {
		return nil, err
	}
	return s.ProductLikes(ctx, productID, userID)
}
