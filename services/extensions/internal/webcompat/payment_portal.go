package webcompat

import (
	"context"
	"encoding/json"
	"io"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type PaymentPortal interface {
	PancakePortal(context.Context, int64) (*payments.CustomerPortalLink, error)
	PancakePortalAvailable() bool
}

func RegisterPaymentPortal(mux *http.ServeMux, auth *Authenticator, portal PaymentPortal) {
	if store.Missing(portal) {
		return
	}
	mux.HandleFunc("POST /api/user/payments/pancake/portal", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store, private")
		w.Header().Set("Pragma", "no-cache")
		w.Header().Set("Referrer-Policy", "no-referrer")
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			writePaymentError(w, err)
			return
		}
		// The browser chooses neither customer, merchant, store nor environment.
		raw, readErr := io.ReadAll(io.LimitReader(r.Body, 1))
		if r.URL.RawQuery != "" || readErr != nil || len(raw) != 0 {
			writePaymentError(w, payments.ErrInvalid)
			return
		}
		link, err := portal.PancakePortal(r.Context(), self.GetUserId())
		if err != nil {
			writePaymentError(w, err)
			return
		}
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		_ = json.NewEncoder(w).Encode(map[string]any{"success": true, "message": "", "data": link})
	})
}
