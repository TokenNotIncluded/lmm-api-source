// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
)

type DirectoryLink struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	URL         string `json:"url"`
	Category    string `json:"category"`
	Summary     string `json:"summary"`
	Description string `json:"description"`
	Enabled     bool   `json:"enabled"`
}

var directoryLinkID = regexp.MustCompile(`^[a-zA-Z0-9_-]{1,64}$`)
var directoryCategories = map[string]bool{"chat": true, "research": true, "resources": true, "developer": true, "creative": true, "other": true}

// ValidateDirectoryLinks ports the legacy AIDirectoryLinks option validation.
func ValidateDirectoryLinks(raw []byte) ([]DirectoryLink, error) {
	if len(raw) > 512<<10 || !strings.HasPrefix(strings.TrimSpace(string(raw)), "[") {
		return nil, errors.New("AI directory must be a JSON array")
	}
	var fields []map[string]json.RawMessage
	var links []DirectoryLink
	if json.Unmarshal(raw, &fields) != nil || json.Unmarshal(raw, &links) != nil || links == nil {
		return nil, errors.New("AI directory must be a JSON array of objects")
	}
	if len(links) > 60 {
		return nil, errors.New("AI directory supports at most 60 websites")
	}
	seen := map[string]bool{}
	for i, link := range links {
		for _, key := range []string{"id", "name", "url", "category", "summary", "description", "enabled"} {
			if v, ok := fields[i][key]; !ok || string(v) == "null" {
				return nil, fmt.Errorf("website %d is missing %s", i+1, key)
			}
		}
		if len(fields[i]) != 7 {
			return nil, fmt.Errorf("website %d has unknown fields", i+1)
		}
		if !directoryLinkID.MatchString(link.ID) || seen[link.ID] {
			return nil, fmt.Errorf("website %d has an invalid or duplicate ID", i+1)
		}
		seen[link.ID] = true
		if strings.TrimSpace(link.Name) == "" || len([]rune(link.Name)) > 80 || len([]rune(link.Summary)) > 180 || len([]rune(link.Description)) > 1200 {
			return nil, fmt.Errorf("website %d has invalid text lengths", i+1)
		}
		if !directoryCategories[link.Category] {
			return nil, fmt.Errorf("website %d has an invalid category", i+1)
		}
		u, err := url.Parse(link.URL)
		if len(link.URL) > 2048 || strings.ContainsAny(link.URL, " \t\r\n") || err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" || u.User != nil {
			return nil, fmt.Errorf("website %d must use an absolute HTTP or HTTPS URL without credentials", i+1)
		}
	}
	return links, nil
}

// GET /api/ai-directory: null links means the Web client uses its defaults.
func (h *handlers) directoryLinks(w http.ResponseWriter, r *http.Request) {
	var raw []byte
	err := h.s.db.QueryRowContext(r.Context(), `SELECT links FROM lmm_community.directory_settings WHERE singleton`).Scan(&raw)
	if errors.Is(err, sql.ErrNoRows) {
		respond(w, map[string]any{"links": nil}, nil)
		return
	}
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, map[string]any{"links": json.RawMessage(raw)}, nil)
}

// PUT /api/ai-directory/links replaces the curated list (administrators only).
func (h *handlers) updateDirectoryLinks(w http.ResponseWriter, r *http.Request) {
	actor, err := h.admin(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	var body struct {
		Links json.RawMessage `json:"links"`
	}
	if err = decode(w, r, 600<<10, &body, false); err != nil {
		respond(w, nil, err)
		return
	}
	links, err := ValidateDirectoryLinks(body.Links)
	if err != nil {
		respond(w, nil, &apiError{http.StatusUnprocessableEntity, "AI_DIRECTORY_INVALID_LINKS", err.Error()})
		return
	}
	encoded, _ := json.Marshal(links)
	_, err = h.s.db.ExecContext(r.Context(), `INSERT INTO lmm_community.directory_settings(singleton,links,updated_by,updated_at) VALUES(true,$1::jsonb,$2,$3)
 ON CONFLICT (singleton) DO UPDATE SET links=EXCLUDED.links,updated_by=EXCLUDED.updated_by,updated_at=EXCLUDED.updated_at`, string(encoded), actor.GetUserId(), h.s.now().Unix())
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, map[string]any{"links": links}, nil)
}

type DirectoryAd struct {
	ID               int64   `json:"id"`
	OwnerUserID      int64   `json:"-"`
	Name             string  `json:"name"`
	URL              string  `json:"url"`
	Summary          string  `json:"summary"`
	Description      string  `json:"description"`
	BidCents         int64   `json:"bid_cents"`
	ChargedQuota     int64   `json:"charged_quota"`
	ChargedAmountUSD *string `json:"charged_amount_usd"`
	Status           string  `json:"status"`
	PaidAt           int64   `json:"paid_at"`
	ExpiresAt        int64   `json:"expires_at"`
	HiddenAt         int64   `json:"hidden_at"`
	RefundedAt       int64   `json:"refunded_at"`
	RefundStatus     string  `json:"refund_status"`
	requestID        string
}

const adColumns = `id,owner_user_id,name,url,summary,description,bid_cents,charged_quota,status,paid_at,expires_at,hidden_at,refunded_at,refund_status,request_id`

type rowScanner interface{ Scan(...any) error }

func scanAd(row rowScanner) (DirectoryAd, error) {
	var a DirectoryAd
	err := row.Scan(&a.ID, &a.OwnerUserID, &a.Name, &a.URL, &a.Summary, &a.Description, &a.BidCents, &a.ChargedQuota, &a.Status, &a.PaidAt, &a.ExpiresAt, &a.HiddenAt, &a.RefundedAt, &a.RefundStatus, &a.requestID)
	return a, err
}

func (s *Service) queryAds(ctx context.Context, query string, args ...any) ([]DirectoryAd, error) {
	rows, err := s.db.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	ads := []DirectoryAd{}
	for rows.Next() {
		ad, err := scanAd(rows)
		if err != nil {
			return nil, err
		}
		ads = append(ads, ad)
	}
	return ads, rows.Err()
}

var errAdInvalid = &apiError{http.StatusUnprocessableEntity, "AI_DIRECTORY_AD_INVALID_INPUT", "invalid advertisement details"}

// GET /api/ai-directory/ads: public, active placements by paid weight.
func (h *handlers) listAds(w http.ResponseWriter, r *http.Request) {
	offset := 0
	if raw := r.URL.Query().Get("offset"); raw != "" {
		var err error
		if offset, err = strconv.Atoi(raw); err != nil || offset < 0 || offset > 100_000 {
			respond(w, nil, errAdInvalid)
			return
		}
	}
	const limit = 20
	ads, err := h.s.queryAds(r.Context(), `SELECT `+adColumns+` FROM lmm_community.directory_ads WHERE status='active' AND expires_at>$1 ORDER BY charged_quota DESC,paid_at,id OFFSET $2 LIMIT $3`, h.s.now().Unix(), offset, limit+1)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	more := len(ads) > limit
	if more {
		ads = ads[:limit]
	}
	respond(w, map[string]any{"items": ads, "has_more": more, "next_offset": offset + len(ads)}, nil)
}

// Prices are quoted in Core wallet credits, and placements charge the Core
// wallet. Neither the USD→credit rate nor a charge/refund command is available
// to extensions, so new paid placements stay disabled.
var errAdPaymentUnavailable = &apiError{http.StatusServiceUnavailable, "AI_DIRECTORY_AD_CURRENCY_UNAVAILABLE", "Paid advertisements are temporarily unavailable"}

func (h *handlers) quoteAd(w http.ResponseWriter, r *http.Request) {
	if _, err := h.user(r); err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, nil, errAdPaymentUnavailable)
}

// POST /api/ai-directory/ads only replays an already paid request; a new
// placement fails before any charge.
func (h *handlers) createAd(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	var in struct {
		Name          string `json:"name"`
		URL           string `json:"url"`
		Summary       string `json:"summary"`
		Description   string `json:"description"`
		BidCents      int64  `json:"bid_cents"`
		ExpectedQuota int64  `json:"expected_quota"`
		RequestID     string `json:"request_id"`
	}
	if err = decode(w, r, 8<<10, &in, false); err != nil || !adRequestID.MatchString(in.RequestID) {
		respond(w, nil, errAdInvalid)
		return
	}
	ad, err := scanAd(h.s.db.QueryRowContext(r.Context(), `SELECT `+adColumns+` FROM lmm_community.directory_ads WHERE request_id=$1`, in.RequestID))
	if errors.Is(err, sql.ErrNoRows) {
		respond(w, nil, errAdPaymentUnavailable)
		return
	}
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if ad.OwnerUserID != actor.GetUserId() || ad.Name != strings.TrimSpace(in.Name) || ad.Summary != strings.TrimSpace(in.Summary) ||
		ad.Description != strings.TrimSpace(in.Description) || ad.BidCents != in.BidCents || ad.ChargedQuota != in.ExpectedQuota || ad.URL != strings.TrimSpace(in.URL) {
		respond(w, nil, &apiError{http.StatusConflict, "AI_DIRECTORY_AD_REQUEST_CONFLICT", "advertisement request ID was already used for different details"})
		return
	}
	respond(w, map[string]any{"ad": ad, "created": false, "charged_quota": ad.ChargedQuota}, nil)
}

var adRequestID = regexp.MustCompile(`^[a-zA-Z0-9_-]{16,80}$`)

func (h *handlers) myAds(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	ads, err := h.s.queryAds(r.Context(), `SELECT `+adColumns+` FROM lmm_community.directory_ads WHERE owner_user_id=$1 ORDER BY paid_at DESC,id DESC LIMIT 100`, actor.GetUserId())
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, map[string]any{"items": ads}, nil)
}

// POST /api/ai-directory/ads/{id}/hide withdraws a running placement
// (administrators only). The legacy full refund is a Core wallet credit; it is
// recorded as refund_status=pending_core and never issued here.
func (h *handlers) hideAd(w http.ResponseWriter, r *http.Request) {
	actor, err := h.admin(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil || id <= 0 {
		respond(w, nil, errAdInvalid)
		return
	}
	notFound := &apiError{http.StatusNotFound, "AI_DIRECTORY_AD_NOT_FOUND", "advertisement not found"}
	now := h.s.now().Unix()
	var ad DirectoryAd
	err = h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var e error
		ad, e = scanAd(tx.QueryRowContext(ctx, `SELECT `+adColumns+` FROM lmm_community.directory_ads WHERE id=$1 FOR UPDATE`, id))
		if errors.Is(e, sql.ErrNoRows) {
			return notFound
		}
		if e != nil || ad.Status == "hidden" {
			return e
		}
		if ad.ExpiresAt <= now {
			return notFound
		}
		_, e = tx.ExecContext(ctx, `UPDATE lmm_community.directory_ads SET status='hidden',hidden_at=$2,hidden_by=$3,refund_status='pending_core' WHERE id=$1`, id, now, actor.GetUserId())
		ad.Status, ad.HiddenAt, ad.RefundStatus = "hidden", now, "pending_core"
		return e
	})
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, map[string]any{"ad": ad, "refunded": ad.RefundStatus == "refunded", "refunded_quota": 0, "refund_pending": ad.RefundStatus == "pending_core"}, nil)
}
