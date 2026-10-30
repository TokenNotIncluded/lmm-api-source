// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"errors"
	"sort"
	"strings"
	"time"
)

type Category struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	SortOrder int    `json:"sort_order"`
	Active    bool   `json:"active"`
	CreatedAt int64  `json:"created_at"`
	UpdatedAt int64  `json:"updated_at"`
}

type categoryCatalogue struct {
	Items []Category `json:"items"`
}

func categoryID(id string) bool {
	if len(id) != 36 {
		return false
	}
	for i, c := range id {
		if i == 8 || i == 13 || i == 18 || i == 23 {
			if c != '-' {
				return false
			}
		} else if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return false
		}
	}
	return true
}

func readCategories(tx Tx) (categoryCatalogue, error) {
	var catalogue categoryCatalogue
	err := tx.Get("commands", "@web:categories", &catalogue)
	if errors.Is(err, ErrNotFound) {
		err = nil
	}
	return catalogue, err
}

func (s *Service) categories(ctx context.Context, admin bool, offset, limit int) (map[string]any, error) {
	if offset < 0 || offset > 100000 || !Page(limit) {
		return nil, ErrInvalid
	}
	items := []Category{}
	err := s.repo.Within(ctx, "@web-categories", func(tx Tx) error {
		catalogue, err := readCategories(tx)
		if err != nil {
			return err
		}
		for _, category := range catalogue.Items {
			if admin || category.Active {
				items = append(items, category)
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	sort.Slice(items, func(i, j int) bool {
		if items[i].SortOrder == items[j].SortOrder {
			return items[i].ID < items[j].ID
		}
		return items[i].SortOrder < items[j].SortOrder
	})
	total := len(items)
	start, end := min(offset, total), min(offset+limit, total)
	return map[string]any{"supported": true, "items": items[start:end], "offset": offset, "limit": limit, "has_more": end < total}, nil
}

func (s *Service) PublicCategories(ctx context.Context, offset, limit int) (map[string]any, error) {
	return s.categories(ctx, false, offset, limit)
}

func (s *Service) readCategory(ctx context.Context, id string, active bool) (Category, error) {
	var result Category
	if !categoryID(id) {
		return result, ErrInvalid
	}
	err := s.repo.Within(ctx, "@web-categories", func(tx Tx) error {
		catalogue, err := readCategories(tx)
		if err != nil {
			return err
		}
		for _, category := range catalogue.Items {
			if category.ID == id && (!active || category.Active) {
				result = category
				return nil
			}
		}
		return ErrNotFound
	})
	return result, err
}

func (s *Service) legacyCategories(ctx context.Context, token string, r LegacyRequest) (any, error) {
	auth, ok := s.auth.(administrativeAuthority)
	if !ok {
		return nil, ErrUnavailable
	}
	if _, err := auth.CheckAdmin(ctx, token, false); err != nil {
		return nil, err
	}
	if r.Action == "admin_categories" {
		return s.categories(ctx, true, r.Offset, r.Limit)
	}
	var in struct {
		Name      string `json:"name"`
		SortOrder int    `json:"sort_order"`
		Active    *bool  `json:"active"`
	}
	if legacyDecode(r.Body, &in) != nil || !Text(in.Name, 100) || in.SortOrder < -1000000 || in.SortOrder > 1000000 || in.Active == nil || r.ID != "" && !categoryID(r.ID) {
		return nil, ErrInvalid
	}
	var result Category
	err := s.repo.Within(ctx, "@web-categories", func(tx Tx) error {
		catalogue, err := readCategories(tx)
		if err != nil {
			return err
		}
		position := -1
		for i, category := range catalogue.Items {
			if category.ID == r.ID {
				position = i
			} else if strings.EqualFold(strings.TrimSpace(category.Name), strings.TrimSpace(in.Name)) {
				return ErrConflict
			}
		}
		now := time.Now().Unix()
		if r.ID == "" {
			if len(catalogue.Items) >= 1000 {
				return ErrInvalid
			}
			id, err := legacyRandomID()
			if err != nil {
				return err
			}
			result = Category{ID: id, CreatedAt: now}
		} else {
			if position < 0 {
				return ErrNotFound
			}
			result = catalogue.Items[position]
		}
		result.Name, result.SortOrder, result.Active, result.UpdatedAt = strings.TrimSpace(in.Name), in.SortOrder, *in.Active, now
		if position < 0 {
			catalogue.Items = append(catalogue.Items, result)
		} else {
			catalogue.Items[position] = result
		}
		return tx.Put("commands", "@web:categories", catalogue)
	})
	return result, err
}

func (s *Service) setProductCategory(ctx context.Context, actor Account, r LegacyRequest) (any, error) {
	var in struct {
		CategoryID *string `json:"category_id"`
	}
	if legacyDecode(r.Body, &in) != nil || in.CategoryID == nil {
		return nil, ErrInvalid
	}
	if *in.CategoryID != "" {
		if _, err := s.readCategory(ctx, *in.CategoryID, true); err != nil {
			return nil, err
		}
	}
	row, err := s.LocateProduct(ctx, r.ID)
	if err != nil {
		return nil, err
	}
	if row.Owner != actor {
		return nil, ErrForbidden
	}
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var product Product
		if err := tx.Get("products", r.ID, &product); err != nil {
			return err
		}
		if product.Legacy == nil || product.Legacy.Status == "deleted" {
			return ErrNotFound
		}
		product.Legacy.CategoryID = *in.CategoryID
		product.Legacy.UpdatedAt = time.Now().Unix()
		variants, err := variantsFor(tx, product.ID)
		if err != nil {
			return err
		}
		if err := snapshotLegacy(&product, row.Owner, variants); err != nil {
			return err
		}
		if err := tx.Put("products", product.ID, product); err != nil {
			return err
		}
		row.Product, row.Variants = product, variants
		return nil
	})
	if err != nil {
		return nil, err
	}
	return s.publicLegacyView(row, false)
}
