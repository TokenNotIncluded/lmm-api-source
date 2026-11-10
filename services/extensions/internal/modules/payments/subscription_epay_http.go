package payments

import (
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/proto"
	"net/http"
	"strconv"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// Stable subscription wire JSON: snake_case, decimal-string int64, base64 bytes.
func subscriptionJSON(w http.ResponseWriter, value proto.Message) {
	raw, err := (protojson.MarshalOptions{UseProtoNames: true, EmitUnpopulated: true}).Marshal(value)
	if err != nil {
		subscriptionHTTPError(w, ErrUnavailable)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(raw)
}

func subscriptionHTTPError(w http.ResponseWriter, err error) {
	switch status.Code(err) {
	case codes.Unauthenticated, codes.PermissionDenied:
		err = ErrDenied
	case codes.InvalidArgument:
		err = ErrInvalid
	case codes.NotFound:
		err = ErrNotFound
	case codes.Aborted, codes.AlreadyExists:
		err = ErrConflict
	}
	paymentError(w, err)
}

// Mount on a separate subscription prefix. Host authentication still applies;
// user calls require the forwarded current Core Session, never a web account ID.
func (s *EPaySubscriptions) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("POST /orders", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		var body struct {
			PlanID    int64 `json:"plan_id"`
			ChannelID int64 `json:"channel_id"`
		}
		if decodeUser(w, r, &body) != nil || len(r.Header.Values("Idempotency-Key")) != 1 {
			subscriptionHTTPError(w, ErrInvalid)
			return
		}
		order, err := s.Prepare(r.Context(), user, r.Header.Get("Idempotency-Key"), body.PlanID, body.ChannelID)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, order)
	})
	mux.HandleFunc("GET /orders/{id}", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		order, err := s.Read(r.Context(), user, r.PathValue("id"))
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, order)
	})
	mux.HandleFunc("POST /orders/{id}/confirm", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		checkout, err := s.Confirm(r.Context(), user, r.PathValue("id"))
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, checkout)
	})
	mux.HandleFunc("POST /orders/{id}/cancel", func(w http.ResponseWriter, r *http.Request) {
		user, err := userCredential(r)
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		order, err := s.Cancel(r.Context(), user, r.PathValue("id"))
		if err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		subscriptionJSON(w, order)
	})
	callback := func(w http.ResponseWriter, r *http.Request) {
		channel, err := strconv.ParseInt(r.PathValue("channel"), 10, 64)
		if err != nil {
			subscriptionHTTPError(w, ErrInvalid)
			return
		}
		if err = s.Accept(r.Context(), channel, r); err != nil {
			subscriptionHTTPError(w, err)
			return
		}
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		_, _ = w.Write([]byte("success"))
	}
	mux.HandleFunc("GET /callbacks/{channel}", callback)
	mux.HandleFunc("POST /callbacks/{channel}", callback)
	return mux
}
