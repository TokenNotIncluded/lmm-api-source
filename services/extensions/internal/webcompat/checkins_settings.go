// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Settings remain reachable while the installed checkins plugin is disabled.
// Core authenticates the live root session; service and funding tokens cannot
// substitute for a browser administrator.
func RegisterCheckinSettings(mux RouteRegistrar, auth *Authenticator, s *checkins.Service, plugins PluginController) {
	handle := func(w http.ResponseWriter, r *http.Request) {
		actor, e := auth.Verify(r.Context(), r)
		if e != nil {
			Respond(w, nil, e)
			return
		}
		if actor.GetPlatformLevel() != 6 {
			Respond(w, nil, store.ErrForbidden)
			return
		}
		if r.URL.RawQuery != "" {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		if s == nil {
			w.Header().Set("Content-Type", "application/json; charset=utf-8")
			w.Header().Set("Cache-Control", "no-store")
			w.Header().Set("X-Content-Type-Options", "nosniff")
			w.WriteHeader(http.StatusServiceUnavailable)
			_ = json.NewEncoder(w).Encode(map[string]any{
				"success": false, "code": "CHECKIN_SETTINGS_NOT_ASSEMBLED",
				"message": "Install and configure the checkins module and its dedicated database before managing check-in settings.",
			})
			return
		}
		user := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
		if r.Method == http.MethodGet {
			result, e := s.Settings(r.Context(), user)
			checkinSettingsResponse(w, result, e, store.Missing(plugins) || plugins.IsEnabled("checkins"))
			return
		}
		d := json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096))
		var input checkins.SettingsInput
		d.DisallowUnknownFields()
		var extra any
		if d.Decode(&input) != nil || d.Decode(&extra) != io.EOF {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		result, e := s.SaveSettings(r.Context(), user, actor.GetUserId(), input)
		checkinSettingsResponse(w, result, e, store.Missing(plugins) || plugins.IsEnabled("checkins"))
	}
	mux.HandleFunc("GET /api/option/checkin", handle)
	mux.HandleFunc("PUT /api/option/checkin", handle)
}
func checkinSettingsResponse(w http.ResponseWriter, result checkins.Settings, e error, moduleEnabled bool) {
	switch e {
	case nil:
	case checkins.ErrInvalid:
		e = store.ErrInvalid
	case checkins.ErrConflict:
		e = store.ErrConflict
	case checkins.ErrForbidden:
		e = store.ErrForbidden
	case checkins.ErrUnauthorized:
		e = store.ErrUnauthorized
	default:
		e = store.ErrUnavailable
	}
	Respond(w, struct {
		checkins.Settings
		ModuleEnabled bool `json:"module_enabled"`
	}{result, moduleEnabled}, e)
}
