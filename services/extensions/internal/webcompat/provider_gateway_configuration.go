// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package webcompat

import (
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func registerProviderGatewayConfiguration(mux RouteRegistrar, auth *Authenticator, configuration *payments.ProviderGatewayConfiguration) {
	mux.HandleFunc("GET /api/option/payment-gateways/config", func(w http.ResponseWriter, r *http.Request) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		if self.GetPlatformLevel() != 6 {
			Respond(w, nil, store.ErrForbidden)
			return
		}
		if configuration == nil {
			writePayment(w, http.StatusServiceUnavailable, "PAYMENT_DISABLED", "Payment checkout is not configured.")
			return
		}
		w.Header().Set("Vary", "Authorization")
		checkoutData(w, http.StatusOK, configuration)
	})
}
