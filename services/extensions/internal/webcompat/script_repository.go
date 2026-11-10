// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"encoding/json"
	"io"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func registerScriptRepository(mux *http.ServeMux, auth *Authenticator, service *operations.Service) {
	handle := func(fn func(http.ResponseWriter, *http.Request, int64, *operations.ScriptRepository)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			actor, err := auth.Verify(r.Context(), r)
			if err != nil {
				operationsRespond(w, nil, err)
				return
			}
			if actor.GetUserId() <= 0 || actor.GetUserId() > operations.MaxID || (actor.GetPlatformLevel() != 5 && actor.GetPlatformLevel() != 6) {
				operationsRespond(w, nil, store.ErrForbidden)
				return
			}
			if !operationsQuery(r, false) {
				operationsRespond(w, nil, operations.ErrInvalid)
				return
			}
			if service == nil || service.Repository == nil {
				operationsRespond(w, nil, operations.ErrUnavailable)
				return
			}
			fn(w, r, actor.GetUserId(), service.Repository)
		}
	}
	mux.HandleFunc("GET /api/scripts/repository", handle(func(w http.ResponseWriter, r *http.Request, _ int64, repository *operations.ScriptRepository) {
		settings, err := repository.Settings(r.Context())
		operationsRespond(w, settings, err)
	}))
	mux.HandleFunc("PUT /api/scripts/repository", handle(func(w http.ResponseWriter, r *http.Request, actor int64, repository *operations.ScriptRepository) {
		if ct := r.Header.Get("Content-Type"); ct != "application/json" && ct != "application/json; charset=utf-8" {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096))
		decoder.DisallowUnknownFields()
		var request struct {
			Revision       *int64 `json:"revision"`
			URL            string `json:"repository_url"`
			Branch         string `json:"branch"`
			GitHubKey      string `json:"github_key"`
			ClearGitHubKey bool   `json:"clear_github_key"`
		}
		if decoder.Decode(&request) != nil || request.Revision == nil || decoder.Decode(new(any)) != io.EOF {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		if request.GitHubKey != "" || request.ClearGitHubKey {
			operationsRespond(w, nil, operations.ErrRepositoryCredentials)
			return
		}
		settings, err := repository.Save(r.Context(), actor, operations.ScriptRepositorySettings{Revision: *request.Revision, URL: request.URL, Branch: request.Branch})
		operationsRespond(w, settings, err)
	}))
	mux.HandleFunc("POST /api/scripts/repository/pull", handle(func(w http.ResponseWriter, r *http.Request, actor int64, repository *operations.ScriptRepository) {
		if !operationsEmptyBody(w, r) {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		settings, err := repository.Pull(r.Context(), actor)
		operationsRespond(w, settings, err)
	}))
}
