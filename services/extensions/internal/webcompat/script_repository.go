// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func registerScriptRepository(mux RouteRegistrar, auth *Authenticator, service *operations.Service) {
	handle := func(fn func(http.ResponseWriter, *http.Request, int64, int32, *operations.ScriptRepository)) http.HandlerFunc {
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
			fn(w, r, actor.GetUserId(), actor.GetPlatformLevel(), service.Repository)
		}
	}
	mux.HandleFunc("GET /api/scripts/repository", handle(func(w http.ResponseWriter, r *http.Request, _ int64, _ int32, repository *operations.ScriptRepository) {
		settings, err := repository.Settings(r.Context())
		operationsRespond(w, settings, err)
	}))
	mux.HandleFunc("PUT /api/scripts/repository", handle(func(w http.ResponseWriter, r *http.Request, actor int64, level int32, repository *operations.ScriptRepository) {
		if ct := r.Header.Get("Content-Type"); ct != "application/json" && ct != "application/json; charset=utf-8" {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		var request struct {
			Revision       *int64 `json:"revision"`
			URL            string `json:"repository_url"`
			Branch         string `json:"branch"`
			GitHubKey      string `json:"github_key"`
			ClearGitHubKey bool   `json:"clear_github_key"`
		}
		if supportDecode(w, r, 8192, &request, "revision", "repository_url", "branch", "github_key", "clear_github_key") != nil || request.Revision == nil {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		current, err := repository.Settings(r.Context())
		if err != nil {
			operationsRespond(w, nil, err)
			return
		}
		if (request.GitHubKey != "" || request.ClearGitHubKey || current.GitHubKeySet) && level != 6 {
			operationsRespond(w, nil, store.ErrForbidden)
			return
		}
		settings, err := repository.SaveWithCredentials(r.Context(), actor, operations.ScriptRepositorySettings{Revision: *request.Revision, URL: request.URL, Branch: request.Branch}, request.GitHubKey, request.ClearGitHubKey)
		operationsRespond(w, settings, err)
	}))
	mux.HandleFunc("POST /api/scripts/repository/pull", handle(func(w http.ResponseWriter, r *http.Request, actor int64, level int32, repository *operations.ScriptRepository) {
		if !operationsEmptyBody(w, r) {
			operationsRespond(w, nil, operations.ErrInvalid)
			return
		}
		current, err := repository.Settings(r.Context())
		if err != nil {
			operationsRespond(w, nil, err)
			return
		}
		if current.GitHubKeySet && level != 6 {
			operationsRespond(w, nil, store.ErrForbidden)
			return
		}
		settings, err := repository.PullRevision(r.Context(), actor, current.Revision)
		operationsRespond(w, settings, err)
	}))
}
