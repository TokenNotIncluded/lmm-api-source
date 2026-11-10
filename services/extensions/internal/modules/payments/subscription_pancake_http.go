package payments

import (
	"encoding/json"
	"google.golang.org/protobuf/encoding/protojson"
	"net/http"
	"strconv"
)

func (s *PancakeSubscriptions) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /orders", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		orders, err := s.List(r.Context(), user)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, orders)
	})
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
		order, err := s.Prepare(r.Context(), user, r.Header.Get("Idempotency-Key"), plan, channel)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, order)
	})
	for method, operation := range map[string]string{"GET /orders/{id}": "read", "POST /orders/{id}/confirm": "confirm"} {
		mux.HandleFunc(method, func(w http.ResponseWriter, r *http.Request) {
			user, err := userCredential(r)
			if err != nil {
				subscriptionHTTPError(w, err)
				return
			}
			var view *PancakePurchaseView
			if operation == "confirm" {
				view, err = s.Confirm(r.Context(), user, r.PathValue("id"))
			} else {
				view, err = s.Read(r.Context(), user, r.PathValue("id"))
			}
			if err != nil {
				subscriptionHTTPError(w, err)
				return
			}
			raw, err := (protojson.MarshalOptions{UseProtoNames: true, EmitUnpopulated: true}).Marshal(view.Order)
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
			_ = json.NewEncoder(w).Encode(map[string]any{"order": json.RawMessage(raw), "checkout": checkout})
		})
	}
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
