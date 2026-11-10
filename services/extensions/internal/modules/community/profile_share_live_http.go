// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"fmt"
	"net/http"
	"strings"
	"time"
)

// Root wires the existing public path to this replacement. No parallel API or
// additional share token is introduced. All layouts use the original consent.
func (h *handlers) profileShareLiveSVG(w http.ResponseWriter, r *http.Request) {
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
	var options profileShareSVGOptions
	var aggregate svgOptions
	var top int
	var err error
	switch query.Get("layout") {
	case "aggregate":
		aggregate, err = parseAggregateSVGOptions(query)
	case "models":
		options, top, err = parseProfileShareModelsSVGOptions(query)
	default:
		options, err = parseProfileShareSVGOptions(query)
	}
	if err != nil {
		respond(w, nil, &apiError{http.StatusBadRequest, "PROFILE_SHARE_INVALID", err.Error()})
		return
	}
	share, err := h.s.loadShare(r, "token=$1", token)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if share == nil || (query.Get("layout") == "aggregate" && !share.AggregateUsageEnabled) || (options.Layout == "models" && !share.ModelUsageEnabled) {
		http.NotFound(w, r)
		return
	}
	if ValidateLinkedProfiles(share.LinkedProfiles, h.s.now()) != nil {
		respond(w, nil, errUnavailable)
		return
	}
	// Identity comes from Core. Go share records cannot keep a disabled user's
	// public card alive or grant identity based on a stale local copy.
	users, err := h.s.core.Users(r.Context(), []int64{share.UserID})
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if !users[share.UserID].Active {
		http.NotFound(w, r)
		return
	}
	now := h.s.now()
	var svg string
	if query.Get("layout") == "aggregate" {
		svg = renderAggregateSVG(aggregate, h.s.resolveProfileShareSources(r.Context(), share, aggregate.Period, now))
	} else {
		start := profileSharePeriodStart(options.Period, now)
		bucket := int64(0)
		if options.Layout == "profile" {
			start = now.UTC().Truncate(24*time.Hour).Unix() - 370*86400
			bucket = 86400
		}
		usage, readErr := h.s.core.profileShareUsage(r.Context(), share.UserID, start, now.Unix(), bucket, top)
		if readErr != nil {
			respond(w, nil, &apiError{http.StatusServiceUnavailable, "PROFILE_SHARE_USAGE_UNAVAILABLE", "Usage is unavailable for this period"})
			return
		}
		if !usage.Active {
			http.NotFound(w, r)
			return
		}
		switch options.Layout {
		case "models":
			modelUsage := profileShareModelUsage{Tokens: usage.Totals.Tokens, Requests: usage.Totals.Requests, ModelCount: usage.ModelCount, CreditsPerUSD: usage.CreditsPerUSD, PricingAvailable: usage.PricingAvailable != nil && *usage.PricingAvailable}
			if modelUsage.PricingAvailable {
				modelUsage.Quota = *usage.Totals.Credits
			}
			if usage.CNYPerUSD != nil {
				modelUsage.CNYPerUSD = *usage.CNYPerUSD
			}
			for _, row := range usage.Models {
				public := profileShareModelRow{ModelName: row.ModelName, Tokens: row.Tokens, Requests: row.Requests, PricingAvailable: usage.PricingAvailable != nil && row.Credits != nil}
				if public.PricingAvailable {
					public.Quota = *row.Credits
				}
				modelUsage.Models = append(modelUsage.Models, public)
			}
			svg, err = renderProfileShareModelsSVG(options, modelUsage, start, now.Unix())
		case "profile":
			owner := profileShareOwner{DisplayName: usage.Name, Username: usage.Username, Role: 1}
			switch usage.PlatformRole {
			case "admin":
				owner.Role = 10
			case "superadmin":
				owner.Role = 100
			}
			// Legacy profile displays lifetime requests next to annual token activity.
			// If old Core rejects all-time, retain unknown instead of substituting the
			// annual count. One aggregate read preserves a single snapshot.
			if options.ShowRequests {
				lifetime, e := h.s.core.profileShareUsage(r.Context(), share.UserID, 0, now.Unix(), 0, 0)
				if e == nil && lifetime.Active {
					owner.RequestCount = &lifetime.Totals.Requests
				}
			}
			days := make([]profileShareDay, 0, len(usage.Buckets))
			for _, p := range usage.Buckets {
				days = append(days, profileShareDay{Day: p.Bucket, Tokens: p.Tokens})
			}
			svg = renderProfileShareProfileSVG(options, &owner, days, start)
		default:
			svg = renderProfileShareSVG(options, profileShareUsage{Tokens: usage.Totals.Tokens, Requests: usage.Totals.Requests})
		}
	}
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	// A provider can take seconds. Both revocation and configuration/owner changes
	// invalidate the in-flight result, even when Cursor itself came from cache.
	current, err := h.s.loadShare(r, "token=$1", token)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if !sameProfileShare(share, current) {
		http.NotFound(w, r)
		return
	}
	users, err = h.s.core.Users(r.Context(), []int64{share.UserID})
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if !users[share.UserID].Active {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'")
	w.Header().Set("Access-Control-Allow-Origin", "*")
	w.Header().Set("Content-Type", "image/svg+xml; charset=utf-8")
	_, _ = fmt.Fprint(w, svg)
}
