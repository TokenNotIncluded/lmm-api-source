// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"bytes"
	"encoding/json"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcprelay"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// RegisterSSMCPRelay exposes the buyer's checkout and the separate operator
// action. Core verifies the operator's authority over the frozen seller.
func RegisterSSMCPRelay(mux RouteRegistrar, auth *Authenticator, relay *ssmcprelay.Service) {
	if relay == nil {
		return
	}
	mux.HandleFunc("GET /api/ssmcpd/orders/{id}", func(w http.ResponseWriter, r *http.Request) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		order, err := relay.GetOrder(r.Context(), self.GetUserId(), r.PathValue("id"))
		Respond(w, order, err)
	})
	mux.HandleFunc("POST /api/ssmcpd/orders/{id}/confirm", func(w http.ResponseWriter, r *http.Request) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		var in struct {
			OrderID string `json:"order_id"`
		}
		id := r.PathValue("id")
		if !decodeRelayAction(w, r, &in) || id != in.OrderID {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		order, err := relay.Confirm(r.Context(), self.GetUserId(), bearer(r), id)
		Respond(w, order, err)
	})
	mux.HandleFunc("POST /api/ssmcpd/admin/orders/{id}/refund", func(w http.ResponseWriter, r *http.Request) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		if self.GetPlatformLevel() < 5 {
			Respond(w, nil, store.ErrForbidden)
			return
		}
		var in struct {
			OrderID string `json:"order_id"`
			UserID  int64  `json:"user_id"`
		}
		id := r.PathValue("id")
		if !decodeRelayAction(w, r, &in) || id != in.OrderID || in.UserID <= 0 {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		order, err := relay.RefundExpired(r.Context(), in.UserID, bearer(r), id)
		Respond(w, order, err)
	})
}

func decodeRelayAction(w http.ResponseWriter, r *http.Request, dst any) bool {
	if r.Header.Get("Content-Type") == "" {
		return false
	}
	body, err := storeBody(w, r, 2048, true)
	if err != nil {
		return false
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.DisallowUnknownFields()
	return decoder.Decode(dst) == nil
}
