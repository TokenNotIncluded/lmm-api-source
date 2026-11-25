package webcompat

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcpd"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"net/http"
	"strings"
)

func registerSSMCPDCommerce(mux RouteRegistrar, auth *Authenticator, s *ssmcpd.Service) {
	run := func(fn func(http.ResponseWriter, *http.Request, string, int64) (any, error)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			self, err := auth.Verify(r.Context(), r)
			if err != nil {
				Respond(w, nil, err)
				return
			}
			if r.URL.RawQuery != "" {
				Respond(w, nil, store.ErrInvalid)
				return
			}
			w.Header().Set("Cache-Control", "no-store")
			result, err := fn(w, r, strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), self.GetUserId())
			Respond(w, result, err)
		}
	}
	mux.HandleFunc("GET /api/plugins/ssmcpd/offer", run(func(_ http.ResponseWriter, r *http.Request, token string, user int64) (any, error) {
		return s.Offer(r.Context(), token, user)
	}))
	mux.HandleFunc("GET /api/plugins/ssmcpd/orders/{order}", run(func(_ http.ResponseWriter, r *http.Request, token string, user int64) (any, error) {
		return s.Order(r.Context(), token, user, r.PathValue("order"))
	}))
	mux.HandleFunc("POST /api/plugins/ssmcpd/orders", run(func(w http.ResponseWriter, r *http.Request, token string, user int64) (any, error) {
		var body ssmcpd.PurchaseRequest
		if err := store.Decode(w, r, &body); err != nil {
			return nil, err
		}
		return s.Buy(r.Context(), token, user, body)
	}))
	mux.HandleFunc("POST /api/plugins/ssmcpd/orders/{order}/refund-request", run(func(w http.ResponseWriter, r *http.Request, token string, user int64) (any, error) {
		var in struct {
			Reason string `json:"reason"`
		}
		if err := store.Decode(w, r, &in); err != nil {
			return nil, err
		}
		return s.RequestRefund(r.Context(), token, user, r.PathValue("order"), in.Reason)
	}))
	mux.HandleFunc("POST /api/plugins/ssmcpd/orders/{order}/redeem", run(func(w http.ResponseWriter, r *http.Request, token string, user int64) (any, error) {
		var empty struct{}
		if err := store.Decode(w, r, &empty); err != nil {
			return nil, err
		}
		return s.Redeem(r.Context(), token, user, r.PathValue("order"))
	}))
}
