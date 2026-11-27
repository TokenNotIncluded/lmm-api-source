// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func registerWalletPresetSettings(mux RouteRegistrar, auth *Authenticator, settings payments.WalletPresets) {
	owner := func(w http.ResponseWriter, r *http.Request) bool {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			writePaymentError(w, err)
			return false
		}
		if self.GetPlatformLevel() != 6 {
			Respond(w, nil, store.ErrForbidden)
			return false
		}
		return true
	}
	mux.HandleFunc("GET /api/user/payments/presets", func(w http.ResponseWriter, r *http.Request) {
		if !owner(w, r) {
			return
		}
		overrides, err := settings.Read()
		if err != nil {
			writePaymentError(w, err)
			return
		}
		checkoutData(w, http.StatusOK, overrides)
	})
	mux.HandleFunc("PUT /api/user/payments/presets/{currency}", func(w http.ResponseWriter, r *http.Request) {
		if !owner(w, r) {
			return
		}
		var request struct {
			Amounts []string `json:"amounts_minor"`
		}
		if r.URL.RawQuery != "" || supportDecode(w, r, 2048, &request, "amounts_minor") != nil {
			writePaymentError(w, payments.ErrInvalid)
			return
		}
		if err := settings.Save(r.PathValue("currency"), request.Amounts); err != nil {
			writePaymentError(w, err)
			return
		}
		checkoutData(w, http.StatusOK, request.Amounts)
	})
}
