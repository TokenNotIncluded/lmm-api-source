// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"encoding/json"
	"io"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func registerRSS(mux *http.ServeMux, auth *Authenticator, service *operations.Service) {
	handle := func(level int32, fn func(http.ResponseWriter, *http.Request, int64, *operations.RSS)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			actor, err := auth.Verify(r.Context(), r)
			if err != nil {
				operationsRespond(w, nil, err)
				return
			}
			if actor.GetPlatformLevel() < level || actor.GetUserId() > operations.MaxID {
				operationsRespond(w, nil, store.ErrForbidden)
				return
			}
			if !operationsQuery(r, false) {
				operationsRespond(w, nil, operations.ErrInvalid)
				return
			}
			if service == nil || service.RSS == nil {
				operationsRespond(w, nil, operations.ErrUnavailable)
				return
			}
			fn(w, r, actor.GetUserId(), service.RSS)
		}
	}
	mux.HandleFunc("GET /api/rss", handle(0, func(w http.ResponseWriter, r *http.Request, _ int64, rss *operations.RSS) {
		feeds, err := rss.Read(r.Context())
		operationsRespond(w, map[string]any{"feeds": feeds}, err)
	}))
	mux.HandleFunc("GET /api/rss/settings", handle(5, func(w http.ResponseWriter, r *http.Request, _ int64, rss *operations.RSS) {
		settings, err := rss.Settings(r.Context())
		operationsRespond(w, settings, err)
	}))
	mux.HandleFunc("PUT /api/rss/settings", handle(6, func(w http.ResponseWriter, r *http.Request, actor int64, rss *operations.RSS) {
		if contentType := r.Header.Get("Content-Type"); contentType != "application/json" && contentType != "application/json; charset=utf-8" {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 64<<10))
		decoder.DisallowUnknownFields()
		var request struct {
			Revision *int64                     `json:"revision"`
			Feeds    []operations.RSSFeedConfig `json:"feeds"`
		}
		if decoder.Decode(&request) != nil || request.Revision == nil || decoder.Decode(new(any)) != io.EOF {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		settings, err := rss.Save(r.Context(), actor, operations.RSSSettings{Revision: *request.Revision, Feeds: request.Feeds})
		operationsRespond(w, settings, err)
	}))
}

func rssHTTPPath(path string) bool {
	return path == "/api/rss" || path == "/api/rss/settings"
}
