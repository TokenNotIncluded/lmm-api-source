// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package store

import (
	"context"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
)

// ConfigureAIReview is a construction-time hook. Only Core can classify or
// establish current owner eligibility; the store owns listing state alone.
func (s *Service) ConfigureAIReview(engine *marketreview.Engine) error {
	if s == nil || engine == nil || engine.Policy == nil || engine.Classifier == nil {
		return ErrInvalid
	}
	s.review = engine
	return nil
}

func (s *Service) AIReviewHistory(ctx context.Context, token string, actor Account, id string) ([]marketreview.Record, error) {
	if actor.Kind != "personal" || !actor.Valid() {
		return nil, ErrForbidden
	}
	if _, err := s.check(ctx, token, actor, Read); err != nil {
		return nil, err
	}
	row, err := s.LocateProduct(ctx, id)
	if err != nil {
		return nil, err
	}
	if row.Owner != actor {
		a, ok := s.auth.(administrativeAuthority)
		if !ok {
			return nil, ErrUnavailable
		}
		if _, err := a.CheckAdmin(ctx, token, false); err != nil {
			return nil, err
		}
	}
	if row.Product.Legacy == nil || row.Product.Legacy.Status == "deleted" {
		return nil, ErrNotFound
	}
	return row.Product.AIReview.Records(), nil
}

type aiReviewQueue interface {
	PendingAIReviews(context.Context, int64, int) ([]LocatedProduct, error)
}

func (p *Postgres) PendingAIReviews(ctx context.Context, now int64, limit int) ([]LocatedProduct, error) {
	if limit < 1 || limit > 20 {
		return nil, ErrInvalid
	}
	return p.productRows(ctx, legacyProductSelect+`WHERE p.body->'legacy'->>'status'='pending' AND EXISTS(SELECT 1 FROM jsonb_array_elements(COALESCE(p.body->'ai_review'->'jobs','[]'::jsonb)) j WHERE j->>'token'=p.body->'ai_review'->>'token' AND ((j->>'status'='pending' AND COALESCE((j->>'retry_at')::bigint,0)<=$1) OR (j->>'status'='running' AND COALESCE((j->>'lease_until')::bigint,0)<=$1))) ORDER BY p.id LIMIT $2`, now, limit)
}

// WorkAIReviews runs a bounded batch. The lease is committed before Core I/O;
// crashes preserve attempts and restart the original token, at most three tries.
func (s *Service) WorkAIReviews(ctx context.Context, limit int) error {
	if s == nil || s.review == nil {
		return nil
	}
	queue, ok := s.repo.(aiReviewQueue)
	if !ok {
		return ErrUnavailable
	}
	rows, err := queue.PendingAIReviews(ctx, time.Now().Unix(), limit)
	if err != nil {
		return err
	}
	for _, row := range rows {
		var claim *marketreview.Job
		err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
			var p Product
			if err := tx.Get("products", row.Product.ID, &p); err != nil {
				return err
			}
			if p.Legacy == nil || p.Legacy.Status != "pending" {
				return nil
			}
			claim = p.AIReview.Claim(time.Now().Unix())
			return tx.Put("products", p.ID, p)
		})
		if err != nil {
			return err
		}
		if claim == nil {
			continue
		}
		result, callErr := s.review.Classify(ctx, *claim)
		enabled, ownerErr := s.review.Classifier.OwnerEnabled(ctx, claim.OwnerID)
		if ownerErr != nil {
			enabled = true
			callErr = &marketreview.Failure{Code: "authority_unavailable", Temporary: true}
		}
		err = s.review.With(ctx, func(modes marketreview.Modes) error {
			return s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
				var p Product
				if err := tx.Get("products", row.Product.ID, &p); err != nil {
					return err
				}
				var shop Shop
				if err := tx.Get("shops", row.ShopID, &shop); err != nil {
					return err
				}
				current := p.Legacy != nil && p.Legacy.Status == "pending" && shop.Owner == row.Owner && row.Owner.Kind == "personal" && row.Owner.ID == claim.OwnerID
				manual := ""
				if current {
					terms, err := readLegacyTerms(tx)
					if err != nil {
						return err
					}
					if terms == nil {
						manual = "seller_terms_required"
					}
					if effectiveVisibility(p.Legacy.Visibility, p.Legacy.TestMode) == "private" {
						manual = "private_listing"
					}
				}
				now := time.Now().Unix()
				apply, approve := p.AIReview.Complete(*claim, result, callErr, modes.Store, current, enabled, manual, now)
				if apply {
					p.Published, p.Legacy.Approved = approve, approve
					p.Legacy.Status = "rejected"
					if approve {
						p.Legacy.Status = "published"
					}
					p.Legacy.ReviewedBy, p.Legacy.ReviewedAt, p.Legacy.UpdatedAt = 0, now, now
					p.Legacy.ReviewNote = ""
					variants, err := variantsFor(tx, p.ID)
					if err != nil {
						return err
					}
					if err = snapshotLegacy(&p, row.Owner, variants); err != nil {
						return err
					}
				}
				return tx.Put("products", p.ID, p)
			})
		})
		if err != nil {
			return err
		}
	}
	return nil
}
