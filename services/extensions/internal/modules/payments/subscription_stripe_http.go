package payments

import (
	"encoding/json"
	"net/http"
	"strconv"

	"google.golang.org/protobuf/encoding/protojson"
)

// Mount under /subscriptions/stripe, separate from wallet Stripe ingress.
// The app wrapper must retain existing Session/host/admission/callback guards.
func (s *StripeSubscriptions) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("POST /orders", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		var body struct {
			PlanID    string `json:"plan_id"`
			ChannelID string `json:"channel_id"`
		}
		if decodeUser(w, r, &body) != nil || len(r.Header.Values("Idempotency-Key")) != 1 {
			subscriptionHTTPError(w, ErrInvalid)
			return
		}
		plan, e1 := strconv.ParseInt(body.PlanID, 10, 64)
		channel, e2 := strconv.ParseInt(body.ChannelID, 10, 64)
		if e1 != nil || e2 != nil || plan <= 0 || channel <= 0 {
			subscriptionHTTPError(w, ErrInvalid)
			return
		}
		o, err := s.Prepare(r.Context(), user, r.Header.Get("Idempotency-Key"), plan, channel)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, o)
	})
	for method, operation := range map[string]string{"GET /orders/{id}": "read", "POST /orders/{id}/confirm": "confirm"} {
		operation := operation
		mux.HandleFunc(method, func(w http.ResponseWriter, r *http.Request) {
			user, err := userCredential(r)
			if err != nil {
				subscriptionHTTPError(w, err)
				return
			}
			var view *StripePurchaseView
			if operation == "confirm" {
				view, err = s.Confirm(r.Context(), user, r.PathValue("id"))
			} else {
				view, err = s.Read(r.Context(), user, r.PathValue("id"))
			}
			if err != nil {
				subscriptionHTTPError(w, err)
				return
			}
			encoded, err := (protojson.MarshalOptions{UseProtoNames: true, EmitUnpopulated: true}).Marshal(view.Order)
			if err != nil {
				subscriptionHTTPError(w, ErrUnavailable)
				return
			}
			var checkout any
			if c := view.Checkout; c != nil {
				checkout = map[string]string{"id": c.ID, "url": c.URL, "expires_at": strconv.FormatInt(c.ExpiresAt, 10)}
			}
			w.Header().Set("Cache-Control", "no-store")
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{"order": json.RawMessage(encoded), "checkout": checkout})
		})
	}
	mux.HandleFunc("POST /orders/{id}/cancel-renewal", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err == nil {
			err = s.CancelRenewal(r.Context(), user, r.PathValue("id"))
		}
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		// The provider mutation was requested; no synthetic Core canceled/paid state.
		w.Header().Set("Cache-Control", "no-store")
		w.WriteHeader(http.StatusAccepted)
	})
	mux.HandleFunc("POST /callbacks/{channel}", func(w http.ResponseWriter, r *http.Request) {
		channel, err := strconv.ParseInt(r.PathValue("channel"), 10, 64)
		if err != nil || channel <= 0 {
			subscriptionHTTPError(w, ErrInvalid)
			return
		}
		receipt, err := s.Accept(r.Context(), channel, r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, receipt)
	})
	return mux
}
