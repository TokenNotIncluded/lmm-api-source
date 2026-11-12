// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type productAIReviews interface {
	AIReviewHistory(context.Context, string, store.Account, string) ([]marketreview.Record, error)
}
type toolAIReviews interface {
	AIReviewHistory(context.Context, string, string, string, string) ([]marketreview.Record, error)
}

func reviewError(err error) error {
	switch {
	case errors.Is(err, marketreview.ErrInvalid):
		return store.ErrInvalid
	case errors.Is(err, marketreview.ErrForbidden):
		return store.ErrForbidden
	case err != nil:
		return store.ErrUnavailable
	default:
		return nil
	}
}

func RegisterMarketAIReview(mux RouteRegistrar, auth *Authenticator, engine *marketreview.Engine) {
	for _, method := range []string{"GET", "PUT"} {
		mux.HandleFunc(method+" /api/security/market-ai-review/settings", func(w http.ResponseWriter, r *http.Request) {
			if _, err := auth.Verify(r.Context(), r); err != nil {
				Respond(w, nil, err)
				return
			}
			credential := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
			if r.Method == http.MethodGet {
				out, err := engine.Settings(r.Context(), credential)
				Respond(w, out, reviewError(err))
				return
			}
			body, err := storeBody(w, r, 4096, true)
			if err != nil {
				Respond(w, nil, err)
				return
			}
			out, err := engine.SaveSettings(r.Context(), credential, body)
			Respond(w, out, reviewError(err))
		})
	}
}

func registerProductAIReviews(mux RouteRegistrar, auth *Authenticator, products StoreService) {
	mux.HandleFunc("GET /api/store/products/{id}/ai-reviews", func(w http.ResponseWriter, r *http.Request) {
		actor, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		port, ok := products.(productAIReviews)
		if !ok {
			Respond(w, nil, store.ErrUnavailable)
			return
		}
		if len(r.URL.Query()) != 0 {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		rows, err := port.AIReviewHistory(r.Context(), strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), store.Account{Kind: "personal", ID: actor.GetUserId()}, r.PathValue("id"))
		Respond(w, map[string]any{"rows": rows}, err)
	})
}

func registerToolAIReviews(mux RouteRegistrar, auth *Authenticator, tools ToolMarketService) {
	mux.HandleFunc("GET /api/tool-market/services/{id}/ai-reviews", func(w http.ResponseWriter, r *http.Request) {
		actor, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		port, ok := tools.(toolAIReviews)
		if !ok {
			Respond(w, nil, store.ErrUnavailable)
			return
		}
		query := r.URL.Query()
		if len(query) > 1 || len(query["version_id"]) > 1 || len(query) == 1 && query.Get("version_id") == "" {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		rows, err := port.AIReviewHistory(r.Context(), strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), strconv.FormatInt(actor.GetUserId(), 10), r.PathValue("id"), query.Get("version_id"))
		toolMarketRespond(w, map[string]any{"rows": rows}, err)
	})
}
