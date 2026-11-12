// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/herosms"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"net/http"
	"strings"
)

func RegisterHeroSMS(mux RouteRegistrar, auth *Authenticator, service *herosms.Service) {
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if _, err := auth.Verify(r.Context(), r); err != nil {
			Respond(w, nil, err)
			return
		}
		if service == nil {
			Respond(w, nil, store.ErrUnavailable)
			return
		}
		request := r.Clone(r.Context())
		request.Header.Set("X-LMM-User-Credential", strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "))
		service.ServeHTTP(w, request)
	})
	mux.Handle("/api/hero-sms/", handler)
	mux.Handle("/api/option/hero-sms", handler)
	mux.Handle("/api/option/hero-sms/", handler)
}
