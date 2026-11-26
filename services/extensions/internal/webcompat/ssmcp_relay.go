// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcprelay"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"io"
	"mime"
	"net/http"
	"strings"
)

func registerSSMCPRelay(mux RouteRegistrar, auth *Authenticator, relay *ssmcprelay.Service) {
	if relay == nil {
		return
	}
	handle := func(w http.ResponseWriter, r *http.Request) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		id := r.PathValue("id")
		if r.Method == http.MethodGet {
			order, err := relay.GetOrder(r.Context(), self.GetUserId(), id)
			Respond(w, order, err)
			return
		}
		var in struct {
			OrderID string `json:"order_id"`
		}
		mt, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
		if err != nil || mt != "application/json" {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, 2048))
		dec.DisallowUnknownFields()
		if dec.Decode(&in) != nil || in.OrderID != id || dec.Decode(new(any)) != io.EOF {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		order, err := relay.Confirm(r.Context(), self.GetUserId(), strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), id)
		Respond(w, order, err)
	}
	mux.HandleFunc("GET /api/ssmcpd/orders/{id}", handle)
	mux.HandleFunc("POST /api/ssmcpd/orders/{id}/confirm", handle)
}
