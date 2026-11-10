// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"encoding/json"
	"net/http"
	"sync"
	"time"
)

// The existing private community port is the sole authority for LMM usage.
// Optional monetary proof/FX may be added to this read DTO without a schema
// change; old Core replies remain unknown, rather than inventing a zero price.
type profileShareCoreUsage = CoreUserUsage

func (c *CoreRead) profileShareUsage(ctx context.Context, userID, start, end, bucket int64, modelLimit int) (profileShareCoreUsage, error) {
	var out profileShareCoreUsage
	err := c.call(ctx, "usage/user", map[string]int64{"user_id": userID, "start": start, "end": end, "bucket_seconds": bucket, "model_limit": int64(modelLimit)}, &out)
	if err != nil {
		return out, err
	}
	valid := func(n int64) bool { return n >= 0 && n <= profileShareMaxMetric }
	validMoney := func(n *int64) bool { return n == nil || valid(*n) }
	pricingKnown := out.PricingAvailable != nil && *out.PricingAvailable
	if out.UserID != userID || !valid(out.Totals.Tokens) || !valid(out.Totals.Requests) || !validMoney(out.Totals.Credits) || (pricingKnown && out.Totals.Credits == nil) || out.CreditsPerUSD <= 0 || len(out.Models) > modelLimit || out.ModelCount < int64(len(out.Models)) || len(out.Buckets) > 1000 {
		return out, errCoreRead
	}
	for _, p := range out.Buckets {
		if bucket <= 0 || p.Bucket%bucket != 0 || p.Bucket < start-start%bucket || p.Bucket > end || !valid(p.Tokens) || !valid(p.Requests) {
			return out, errCoreRead
		}
	}
	var tokens, requests, credits int64
	for _, p := range out.Models {
		if !valid(p.Tokens) || !valid(p.Requests) || !validMoney(p.Credits) || (pricingKnown && p.Credits == nil) {
			return out, errCoreRead
		}
		tokens += p.Tokens
		requests += p.Requests
		if p.Credits != nil {
			credits += *p.Credits
		}
	}
	if tokens > out.Totals.Tokens || requests > out.Totals.Requests || (pricingKnown && credits > *out.Totals.Credits) {
		return out, errCoreRead
	}
	return out, nil
}

// Resolve from current Core aggregates and anonymous public Cursor IO, not a
// browser-provided snapshot. Explicit imported snapshots retain their source.
func (s *Service) resolveProfileShareSources(ctx context.Context, share *profileShare, period string, now time.Time) []aggregateSource {
	rows := s.aggregateSources(share, period)
	if !share.AggregateUsageEnabled {
		return rows
	}
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	start := profileSharePeriodStart(period, now)
	native := &rows[0]
	if start > 0 {
		native.PeriodStart = time.Unix(start, 0).UTC().Format(time.RFC3339)
	}
	native.PeriodEnd = now.UTC().Format(time.RFC3339)
	usage, err := s.core.profileShareUsage(ctx, share.UserID, start, now.Unix(), 0, 0)
	if err == nil && usage.Active {
		native.Status, native.Reason = "live", ""
		native.Tokens, native.Requests = &usage.Totals.Tokens, &usage.Totals.Requests
		native.ObservedAt = now.UTC().Format(time.RFC3339)
	}
	var wg sync.WaitGroup
	for i, p := range share.LinkedProfiles {
		if p.Provider != "cursor" || p.Snapshot != nil {
			continue
		}
		wg.Add(1)
		go func(index int, p LinkedProfile) {
			defer wg.Done()
			row := fetchProfileCursor(ctx, p.URL, now)
			row.Label = rows[index].Label
			rows[index] = row
		}(i+1, p)
	}
	wg.Wait()
	return rows
}

func sameProfileShare(a, b *profileShare) bool {
	if a == nil || b == nil {
		return a == nil && b == nil
	}
	if a.UserID != b.UserID || a.Token != b.Token || a.ModelUsageEnabled != b.ModelUsageEnabled || a.AggregateUsageEnabled != b.AggregateUsageEnabled {
		return false
	}
	x, _ := json.Marshal(a.LinkedProfiles)
	y, _ := json.Marshal(b.LinkedProfiles)
	return string(x) == string(y)
}

// Root replaces the existing state-response call sites with this method.
// Recheck the original Session and persisted consent after provider IO.
func (h *handlers) profileShareLiveState(w http.ResponseWriter, r *http.Request, share *profileShare) {
	w.Header().Set("Cache-Control", "no-store")
	if share == nil {
		respond(w, map[string]any{"enabled": false}, nil)
		return
	}
	if ValidateLinkedProfiles(share.LinkedProfiles, h.s.now()) != nil {
		respond(w, nil, errUnavailable)
		return
	}
	sources := h.s.resolveProfileShareSources(r.Context(), share, "30d", h.s.now())
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	current, err := h.s.loadShare(r, "user_id=$1", actor.GetUserId())
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if current == nil {
		respond(w, map[string]any{"enabled": false}, nil)
		return
	}
	if actor.GetUserId() != share.UserID || !sameProfileShare(share, current) {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, map[string]any{
		"enabled": true, "model_usage_enabled": current.ModelUsageEnabled, "aggregate_usage_enabled": current.AggregateUsageEnabled,
		"linked_profiles": current.LinkedProfiles, "aggregate_sources": sources, "token": current.Token, "url": h.s.origin + "/api/share/profile/" + current.Token + ".svg",
	}, nil)
}
