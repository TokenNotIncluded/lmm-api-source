// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"errors"
	"net/url"
	"strings"
	"unicode/utf8"
)

const merchantHomeKey = "@web:merchant-home"
const storeAnnouncementKey = "@web:store-announcement"

type merchantHome struct {
	Biography    string `json:"biography"`
	Announcement string `json:"announcement"`
	HeaderImage  string `json:"header_image"`
	Version      int64  `json:"version"`
}

type storeAnnouncement struct {
	Content string `json:"content"`
}

type merchantShopIndex interface {
	HasPersonalShop(context.Context, int64) (bool, error)
}

func (s *Service) merchantHomeReady() bool {
	_, administrative := s.auth.(administrativeAuthority)
	_, indexed := s.repo.(merchantShopIndex)
	return administrative && indexed && !Missing(s.repo) && !Missing(s.auth)
}

func validMerchantText(value string, limit int) bool {
	return utf8.ValidString(value) && !strings.ContainsRune(value, 0) && utf8.RuneCountInString(value) <= limit
}

func validMerchantHome(row merchantHome) bool {
	if !validMerchantText(row.Biography, 4096) || !validMerchantText(row.Announcement, 16384) || row.Version < 0 || row.Version > legacySafeInteger {
		return false
	}
	if row.HeaderImage == "" {
		return true
	}
	// Header images are browser resources, never fetched by the extension.
	// The existing SVG capability remains disabled.
	if !validMerchantText(row.HeaderImage, 4096) {
		return false
	}
	image, err := url.Parse(row.HeaderImage)
	return err == nil && image.Scheme == "https" && image.Hostname() != "" && image.User == nil && image.Opaque == ""
}

func readMerchantHome(tx Tx) (merchantHome, error) {
	var row merchantHome
	err := tx.Get("commands", merchantHomeKey, &row)
	if errors.Is(err, ErrNotFound) {
		return merchantHome{}, nil
	}
	if err != nil {
		return merchantHome{}, err
	}
	if !validMerchantHome(row) || row.Version == 0 {
		return merchantHome{}, ErrUnavailable
	}
	return row, nil
}

func merchantHomeView(actor Account, row merchantHome) map[string]any {
	// Core's identity contract supplies an account ID, not public profile data.
	// Empty names are explicit; do not fabricate usernames or disclose email.
	return map[string]any{
		"seller":    map[string]any{"id": actor.ID, "username": "", "display_name": ""},
		"biography": row.Biography, "announcement": row.Announcement,
		"header_image": row.HeaderImage, "version": row.Version,
	}
}

// PublicMerchantHome reads an existing shop without creating one for an
// arbitrary public account ID. An established empty shop has an empty profile.
func (s *Service) PublicMerchantHome(ctx context.Context, sellerID int64) (map[string]any, error) {
	if sellerID <= 0 || sellerID > legacySafeInteger {
		return nil, ErrInvalid
	}
	if !s.merchantHomeReady() {
		return nil, ErrUnavailable
	}
	actor := Account{Kind: "personal", ID: sellerID}
	shopID := ID("legacy-web-shop-v1", actor)
	var out map[string]any
	err := s.repo.Within(ctx, shopID, func(tx Tx) error {
		var shop Shop
		if err := tx.Get("shops", shopID, &shop); err != nil {
			return err
		}
		if shop.ID != shopID || shop.Owner != actor {
			return ErrUnavailable
		}
		row, err := readMerchantHome(tx)
		if err != nil {
			return err
		}
		out = merchantHomeView(actor, row)
		return nil
	})
	if errors.Is(err, ErrNotFound) {
		// Native catalogue shops predate the Web profile. Their real owners
		// still have a public empty home; arbitrary account IDs do not.
		exists, lookupErr := s.repo.(merchantShopIndex).HasPersonalShop(ctx, sellerID)
		if lookupErr != nil {
			return nil, lookupErr
		}
		if exists {
			return merchantHomeView(actor, merchantHome{}), nil
		}
	}
	return out, err
}

// legacyMerchantHome is reached only after Legacy checks the live Core actor's
// ownership. The existing per-shop transaction lock serializes the version CAS.
func (s *Service) legacyMerchantHome(ctx context.Context, actor Account, r LegacyRequest) (any, error) {
	if !s.merchantHomeReady() {
		return nil, ErrUnavailable
	}
	var input struct {
		Biography       string `json:"biography"`
		Announcement    string `json:"announcement"`
		HeaderImage     string `json:"header_image"`
		ExpectedVersion *int64 `json:"expected_version"`
	}
	if r.Action == "save_home" {
		if legacyDecode(r.Body, &input) != nil || input.ExpectedVersion == nil {
			return nil, ErrInvalid
		}
		if !validMerchantHome(merchantHome{Biography: input.Biography, Announcement: input.Announcement, HeaderImage: input.HeaderImage, Version: *input.ExpectedVersion}) {
			return nil, ErrInvalid
		}
	}
	shopID := ID("legacy-web-shop-v1", actor)
	var out map[string]any
	err := s.repo.Within(ctx, shopID, func(tx Tx) error {
		var shop Shop
		shopErr := tx.Get("shops", shopID, &shop)
		if shopErr != nil && !errors.Is(shopErr, ErrNotFound) {
			return shopErr
		}
		if shopErr == nil && (shop.ID != shopID || shop.Owner != actor) {
			return ErrUnavailable
		}
		row, err := readMerchantHome(tx)
		if err != nil {
			return err
		}
		if r.Action == "save_home" {
			if row.Version != *input.ExpectedVersion || row.Version == legacySafeInteger {
				return ErrConflict
			}
			if errors.Is(shopErr, ErrNotFound) {
				shop = Shop{ID: shopID, Owner: actor, Title: "Merchant store"}
				if err := tx.Put("shops", shopID, shop); err != nil {
					return err
				}
			}
			row = merchantHome{Biography: input.Biography, Announcement: input.Announcement, HeaderImage: input.HeaderImage, Version: row.Version + 1}
			if err := tx.Put("commands", merchantHomeKey, row); err != nil {
				return err
			}
		}
		out = merchantHomeView(actor, row)
		return nil
	})
	return out, err
}

func (s *Service) PublicAnnouncement(ctx context.Context) (map[string]any, error) {
	if !s.merchantHomeReady() {
		return nil, ErrUnavailable
	}
	var row storeAnnouncement
	err := s.repo.Within(ctx, "@web-legal", func(tx Tx) error {
		err := tx.Get("commands", storeAnnouncementKey, &row)
		if errors.Is(err, ErrNotFound) {
			return nil
		}
		if err != nil {
			return err
		}
		if !validMerchantText(row.Content, 16384) {
			return ErrUnavailable
		}
		return nil
	})
	return map[string]any{"content": row.Content}, err
}

func (s *Service) saveStoreAnnouncement(ctx context.Context, token string, r LegacyRequest) (any, error) {
	if !s.merchantHomeReady() {
		return nil, ErrUnavailable
	}
	auth := s.auth.(administrativeAuthority)
	if _, err := auth.CheckAdmin(ctx, token, false); err != nil {
		return nil, err
	}
	var row storeAnnouncement
	if legacyDecode(r.Body, &row) != nil || !validMerchantText(row.Content, 16384) {
		return nil, ErrInvalid
	}
	err := s.repo.Within(ctx, "@web-legal", func(tx Tx) error {
		return tx.Put("commands", storeAnnouncementKey, row)
	})
	return map[string]any{"content": row.Content}, err
}
