// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"path"
	"regexp"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

const profileShareMaxLinkedProfiles = 5
const profileShareMaxMetric int64 = 1<<53 - 1

var profileShareToken = regexp.MustCompile(`^[0-9a-f]{48}$`)
var profileShareHandle = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$`)
var profileShareHost = regexp.MustCompile(`^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$`)

type ProfileUsageSnapshot struct {
	Tokens      *int64 `json:"tokens,omitempty"`
	Requests    *int64 `json:"requests,omitempty"`
	Messages    *int64 `json:"messages,omitempty"`
	Period      string `json:"period"`
	PeriodStart string `json:"period_start,omitempty"`
	PeriodEnd   string `json:"period_end,omitempty"`
	ObservedAt  string `json:"observed_at"`
	Approximate bool   `json:"approximate"`
	Source      string `json:"source,omitempty"`
}

type LinkedProfile struct {
	Provider string                `json:"provider"`
	URL      string                `json:"url"`
	Label    string                `json:"label,omitempty"`
	Snapshot *ProfileUsageSnapshot `json:"snapshot,omitempty"`
}

type profileShare struct {
	UserID                int64
	Token                 string
	CreatedAt             int64
	ModelUsageEnabled     bool
	AggregateUsageEnabled bool
	LinkedProfiles        []LinkedProfile
}

func cleanText(value string, maxRunes int) (string, error) {
	value = strings.TrimSpace(value)
	if utf8.RuneCountInString(value) > maxRunes {
		return "", errors.New("text is too long")
	}
	for _, r := range value {
		if unicode.IsControl(r) {
			return "", errors.New("text contains control characters")
		}
	}
	return value, nil
}

func validateLinkedURL(provider, raw string) error {
	if len(raw) > 512 {
		return errors.New("profile URL is too long")
	}
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "https" || u.Opaque != "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.RawPath != "" || u.Port() != "" || u.Host != strings.ToLower(u.Hostname()) {
		return errors.New("profile URL must use canonical HTTPS without credentials, ports or query")
	}
	host := u.Hostname()
	if !profileShareHost.MatchString(host) || net.ParseIP(host) != nil {
		return errors.New("profile URL must use a public hostname")
	}
	for _, suffix := range []string{".local", ".localhost", ".internal", ".lan", ".home", ".arpa", ".onion", ".test", ".invalid", ".example"} {
		if strings.HasSuffix(host, suffix) {
			return errors.New("profile URL must use a public hostname")
		}
	}
	var handle string
	switch provider {
	case "cursor":
		if host != "cursor.com" || !strings.HasPrefix(u.Path, "/@") {
			return errors.New("Cursor URL must be https://cursor.com/@handle")
		}
		handle = strings.TrimPrefix(u.Path, "/@")
	case "chatgpt":
		if host != "chatgpt.com" || !strings.HasPrefix(u.Path, "/u/") {
			return errors.New("ChatGPT URL must be https://chatgpt.com/u/handle")
		}
		handle = strings.TrimPrefix(u.Path, "/u/")
	case "custom":
		if u.Path == "" || u.Path == "/" || path.Clean(u.Path) != u.Path || strings.ContainsAny(u.Path, "%\\") {
			return errors.New("invalid profile path")
		}
		if _, err := cleanText(u.Path, 512); err != nil || u.String() != raw {
			return errors.New("profile URL must be canonical")
		}
		return nil
	default:
		return errors.New("unsupported profile provider")
	}
	if !profileShareHandle.MatchString(handle) || u.String() != raw {
		return errors.New("invalid profile handle or path")
	}
	return nil
}

// ValidateLinkedProfiles ports the legacy owner-supplied profile validation and
// normalizes labels and observation times in place.
func ValidateLinkedProfiles(profiles []LinkedProfile, now time.Time) error {
	if len(profiles) > profileShareMaxLinkedProfiles {
		return errors.New("at most five linked profiles are allowed")
	}
	seen := map[string]bool{}
	for i := range profiles {
		p := &profiles[i]
		if err := validateLinkedURL(p.Provider, p.URL); err != nil {
			return err
		}
		if seen[p.URL] {
			return errors.New("duplicate profile URL")
		}
		seen[p.URL] = true
		var err error
		if p.Label, err = cleanText(p.Label, 48); err != nil {
			return errors.New("invalid profile label")
		}
		s := p.Snapshot
		if s == nil {
			continue
		}
		if s.Tokens == nil && s.Requests == nil && s.Messages == nil {
			return errors.New("snapshot requires a known metric")
		}
		for _, v := range []*int64{s.Tokens, s.Requests, s.Messages} {
			if v != nil && (*v < 0 || *v > profileShareMaxMetric) {
				return errors.New("snapshot metrics must be non-negative safe integers")
			}
		}
		observed, err := time.Parse(time.RFC3339, s.ObservedAt)
		if err != nil || observed.Before(time.Date(2000, 1, 1, 0, 0, 0, 0, time.UTC)) || observed.After(now.Add(5*time.Minute)) {
			return errors.New("invalid snapshot observation time")
		}
		s.ObservedAt = observed.UTC().Format(time.RFC3339)
		if s.Source, err = cleanText(s.Source, 80); err != nil {
			return errors.New("invalid snapshot source")
		}
		switch s.Period {
		case "all", "7d", "30d", "365d", "custom":
		default:
			return errors.New("invalid snapshot period")
		}
		if (s.PeriodStart == "") != (s.PeriodEnd == "") || (s.Period == "custom" && s.PeriodStart == "") || (s.Period == "all" && s.PeriodStart != "") {
			return errors.New("invalid snapshot date range")
		}
		if s.PeriodStart != "" {
			start, e1 := time.Parse("2006-01-02", s.PeriodStart)
			end, e2 := time.Parse("2006-01-02", s.PeriodEnd)
			maxDays := 365
			if s.Period == "7d" {
				maxDays = 7
			} else if s.Period == "30d" {
				maxDays = 30
			}
			if e1 != nil || e2 != nil || end.Before(start) || end.Sub(start) > time.Duration(maxDays)*24*time.Hour || s.PeriodEnd > observed.UTC().Format("2006-01-02") {
				return errors.New("invalid snapshot date range")
			}
		}
	}
	return nil
}

type aggregateSource struct {
	Provider       string `json:"provider"`
	URL            string `json:"url"`
	Label          string `json:"label"`
	Status         string `json:"status"`
	Tokens         *int64 `json:"tokens,omitempty"`
	Requests       *int64 `json:"requests,omitempty"`
	Messages       *int64 `json:"messages,omitempty"`
	Period         string `json:"period"`
	PeriodStart    string `json:"period_start,omitempty"`
	PeriodEnd      string `json:"period_end,omitempty"`
	PeriodTimezone string `json:"period_timezone,omitempty"`
	ObservedAt     string `json:"observed_at,omitempty"`
	Approximate    bool   `json:"approximate"`
	Source         string `json:"source"`
	SnapshotSource string `json:"snapshot_source,omitempty"`
	Reason         string `json:"reason,omitempty"`
}

// aggregateSources shows owner snapshots only. Native LMM usage needs a Core
// per-user usage read, and the legacy live Cursor page fetch is not ported.
func (s *Service) aggregateSources(share *profileShare, period string) []aggregateSource {
	rows := []aggregateSource{{Provider: "lmm", URL: s.origin, Label: "LMM Best", Status: "disabled", Period: period, Source: "native", PeriodTimezone: "UTC"}}
	for _, p := range share.LinkedProfiles {
		label := p.Label
		if label == "" {
			label = map[string]string{"cursor": "Cursor", "chatgpt": "ChatGPT"}[p.Provider]
			if label == "" {
				label = "Custom profile"
			}
		}
		row := aggregateSource{Provider: p.Provider, URL: p.URL, Label: label, Status: "disabled", Period: "unknown", Source: "owner_snapshot"}
		if share.AggregateUsageEnabled {
			switch {
			case p.Snapshot != nil:
				sn := p.Snapshot
				row.Status, row.Period = "snapshot", sn.Period
				row.Tokens, row.Requests, row.Messages = sn.Tokens, sn.Requests, sn.Messages
				row.PeriodStart, row.PeriodEnd, row.ObservedAt = sn.PeriodStart, sn.PeriodEnd, sn.ObservedAt
				row.Approximate, row.SnapshotSource = sn.Approximate, sn.Source
			case p.Provider == "chatgpt":
				row.Status, row.Reason = "login_required", "anonymous_metrics_unavailable"
			case p.Provider == "custom":
				row.Status, row.Reason = "unsupported", "snapshot_required"
			default:
				row.Status, row.Reason, row.Source = "unavailable", "public_fetch_unavailable", "public_ssr"
			}
		}
		rows = append(rows, row)
	}
	if share.AggregateUsageEnabled {
		rows[0].Status, rows[0].Reason = "unavailable", "usage_unavailable"
	}
	return rows
}

func (s *Service) loadShare(r *http.Request, query string, arg any) (*profileShare, error) {
	var sh profileShare
	var linked []byte
	err := s.db.QueryRowContext(r.Context(), `SELECT user_id,token,created_at,model_usage_enabled,aggregate_usage_enabled,linked_profiles FROM lmm_community.profile_shares WHERE `+query, arg).
		Scan(&sh.UserID, &sh.Token, &sh.CreatedAt, &sh.ModelUsageEnabled, &sh.AggregateUsageEnabled, &linked)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if err = json.Unmarshal(linked, &sh.LinkedProfiles); err != nil {
		return nil, err
	}
	if sh.LinkedProfiles == nil {
		sh.LinkedProfiles = []LinkedProfile{}
	}
	return &sh, nil
}

func (h *handlers) shareState(w http.ResponseWriter, share *profileShare) {
	if share == nil {
		respond(w, map[string]any{"enabled": false}, nil)
		return
	}
	respond(w, map[string]any{
		"enabled": true, "model_usage_enabled": share.ModelUsageEnabled, "aggregate_usage_enabled": share.AggregateUsageEnabled,
		"linked_profiles": share.LinkedProfiles, "aggregate_sources": h.s.aggregateSources(share, "30d"),
		"token": share.Token, "url": h.s.origin + "/api/share/profile/" + share.Token + ".svg",
	}, nil)
}

func (h *handlers) getProfileShare(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	share, err := h.s.loadShare(r, "user_id=$1", actor.GetUserId())
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	h.shareState(w, share)
}

// POST enables sharing (idempotently) and applies only the supplied settings.
func (h *handlers) enableProfileShare(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	var in struct {
		ModelUsageEnabled     *bool            `json:"model_usage_enabled"`
		AggregateUsageEnabled *bool            `json:"aggregate_usage_enabled"`
		LinkedProfiles        *[]LinkedProfile `json:"linked_profiles"`
	}
	if err = decode(w, r, 16<<10, &in, true); err != nil {
		respond(w, nil, &apiError{http.StatusBadRequest, "PROFILE_SHARE_INVALID", "Invalid profile sharing settings"})
		return
	}
	var linked []byte
	if in.LinkedProfiles != nil {
		if *in.LinkedProfiles == nil {
			*in.LinkedProfiles = []LinkedProfile{}
		}
		if err = ValidateLinkedProfiles(*in.LinkedProfiles, h.s.now()); err != nil {
			respond(w, nil, &apiError{http.StatusBadRequest, "PROFILE_SHARE_INVALID", err.Error()})
			return
		}
		linked, _ = json.Marshal(*in.LinkedProfiles)
	}
	token := make([]byte, 24)
	if _, err = rand.Read(token); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	_, err = h.s.db.ExecContext(r.Context(), `INSERT INTO lmm_community.profile_shares(user_id,token,created_at,model_usage_enabled,aggregate_usage_enabled,linked_profiles)
 VALUES($1,$2,$3,COALESCE($4,false),COALESCE($5,false),COALESCE($6::jsonb,'[]'::jsonb))
 ON CONFLICT (user_id) DO UPDATE SET model_usage_enabled=COALESCE($4,lmm_community.profile_shares.model_usage_enabled),
 aggregate_usage_enabled=COALESCE($5,lmm_community.profile_shares.aggregate_usage_enabled),
 linked_profiles=COALESCE($6::jsonb,lmm_community.profile_shares.linked_profiles)`,
		actor.GetUserId(), hex.EncodeToString(token), h.s.now().Unix(), in.ModelUsageEnabled, in.AggregateUsageEnabled, nullableJSON(linked))
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	share, err := h.s.loadShare(r, "user_id=$1", actor.GetUserId())
	if err != nil || share == nil {
		respond(w, nil, errUnavailable)
		return
	}
	h.shareState(w, share)
}

func nullableJSON(b []byte) any {
	if b == nil {
		return nil
	}
	return string(b)
}

// DELETE revokes the public URL; a later enable issues a new token.
func (h *handlers) disableProfileShare(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if _, err = h.s.db.ExecContext(r.Context(), `DELETE FROM lmm_community.profile_shares WHERE user_id=$1`, actor.GetUserId()); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	h.shareState(w, nil)
}

// GET /api/share/profile/{token}.svg is public. The aggregate layout renders
// owner snapshots; layouts built from native LMM usage need a Core read.
func (h *handlers) profileShareSVG(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	token, ok := strings.CutSuffix(r.PathValue("token"), ".svg")
	if !ok || !profileShareToken.MatchString(token) {
		http.NotFound(w, r)
		return
	}
	if len(r.URL.RawQuery) > 2048 {
		respond(w, nil, &apiError{http.StatusBadRequest, "PROFILE_SHARE_INVALID", "SVG options are too long"})
		return
	}
	query := r.URL.Query()
	if query.Get("layout") != "aggregate" {
		respond(w, nil, &apiError{http.StatusServiceUnavailable, "PROFILE_SHARE_USAGE_UNAVAILABLE", "Usage cards require a Core usage read that is not available yet"})
		return
	}
	options, err := parseAggregateSVGOptions(query)
	if err != nil {
		respond(w, nil, &apiError{http.StatusBadRequest, "PROFILE_SHARE_INVALID", err.Error()})
		return
	}
	share, err := h.s.loadShare(r, "token=$1", token)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if share == nil || !share.AggregateUsageEnabled {
		http.NotFound(w, r)
		return
	}
	svg := renderAggregateSVG(options, h.s.aggregateSources(share, options.Period))
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'")
	w.Header().Set("Access-Control-Allow-Origin", "*")
	w.Header().Set("Content-Type", "image/svg+xml; charset=utf-8")
	_, _ = fmt.Fprint(w, svg)
}
