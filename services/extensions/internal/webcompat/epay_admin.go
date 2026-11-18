// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type EPayAdmin interface {
	Configuration(context.Context) (any, error)
}

// Only the current owner can inspect the payment connection. Writes go to
// Core's native owner endpoint so private signing material never enters Go.
func RegisterEPayAdmin(mux RouteRegistrar, auth *Authenticator, admin EPayAdmin) {
	if store.Missing(admin) {
		return
	}
	mux.HandleFunc("GET /api/option/epay/config", func(w http.ResponseWriter, r *http.Request) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		if self.GetPlatformLevel() != 6 {
			Respond(w, nil, store.ErrForbidden)
			return
		}
		value, err := admin.Configuration(r.Context())
		if err != nil {
			writePaymentError(w, err)
			return
		}
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("Cache-Control", "private, no-store")
		w.Header().Set("Vary", "Authorization")
		_ = json.NewEncoder(w).Encode(map[string]any{"success": true, "message": "success", "data": value})
	})
}
