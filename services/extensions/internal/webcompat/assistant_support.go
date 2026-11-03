// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
)

// AssistantSupport is the platform human-support port behind the original
// /api/assistant/support browser routes. It is separate from store support.
type AssistantSupport interface {
	WebSupportEligibility(context.Context, string) (bool, error)
	WebSupportSelf(context.Context, string, int64) (*assistant_services.WebSupportRequest, error)
	WebCreateSupport(context.Context, string, assistant_services.WebSupportInput) (assistant_services.WebSupportRequest, bool, error)
	WebSupportDetail(context.Context, string, int64) (assistant_services.WebSupportRequest, []assistant_services.WebSupportMessage, error)
	WebSendSupportMessage(context.Context, string, int64, string, string) (assistant_services.WebSupportMessage, error)
	WebAcceptSupport(context.Context, string, int64) (assistant_services.WebSupportRequest, error)
	WebCloseSupport(context.Context, string, int64, bool) (assistant_services.WebSupportRequest, error)
}

// RegisterAssistantSupport requires the current browser session for every
// operation; the service rechecks the actor against Core on each call.
func RegisterAssistantSupport(mux *http.ServeMux, auth *Authenticator, support AssistantSupport) {
	protected := func(fn func(http.ResponseWriter, *http.Request, string)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			if _, err := auth.Verify(r.Context(), r); err != nil {
				supportRespond(w, nil, assistantAuthError(err))
				return
			}
			token, _ := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
			fn(w, r, token)
		}
	}
	withID := func(fn func(http.ResponseWriter, *http.Request, string, int64)) http.HandlerFunc {
		return protected(func(w http.ResponseWriter, r *http.Request, token string) {
			id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
			if err != nil || id <= 0 {
				supportRespond(w, nil, access.ErrNotFound)
				return
			}
			fn(w, r, token, id)
		})
	}
	body := func(w http.ResponseWriter, r *http.Request, out any) bool {
		decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 16<<10))
		decoder.DisallowUnknownFields()
		if decoder.Decode(out) != nil || decoder.More() {
			supportRespond(w, nil, access.ErrInvalid)
			return false
		}
		return true
	}
	mux.HandleFunc("GET /api/assistant/support/eligibility", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		eligible, err := support.WebSupportEligibility(r.Context(), token)
		supportRespond(w, map[string]any{"eligible": eligible}, err)
	}))
	mux.HandleFunc("GET /api/assistant/support/self", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		conversation := int64(0)
		if values := r.URL.Query()["conversation_id"]; len(values) > 0 {
			var err error
			conversation, err = strconv.ParseInt(values[0], 10, 64)
			if len(values) != 1 || err != nil || conversation < 0 {
				supportRespond(w, nil, access.ErrInvalid)
				return
			}
		}
		request, err := support.WebSupportSelf(r.Context(), token, conversation)
		supportRespond(w, map[string]any{"request": request}, err)
	}))
	mux.HandleFunc("POST /api/assistant/support", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		var input assistant_services.WebSupportInput
		if !body(w, r, &input) {
			return
		}
		request, created, err := support.WebCreateSupport(r.Context(), token, input)
		supportRespond(w, map[string]any{"request": request, "created": created}, err)
	}))
	mux.HandleFunc("GET /api/assistant/support/{id}", withID(func(w http.ResponseWriter, r *http.Request, token string, id int64) {
		request, messages, err := support.WebSupportDetail(r.Context(), token, id)
		supportRespond(w, map[string]any{"request": request, "messages": messages}, err)
	}))
	mux.HandleFunc("POST /api/assistant/support/{id}/messages", withID(func(w http.ResponseWriter, r *http.Request, token string, id int64) {
		var input struct {
			Content      string `json:"content"`
			ClientTurnID string `json:"client_turn_id,omitempty"`
		}
		if !body(w, r, &input) {
			return
		}
		message, err := support.WebSendSupportMessage(r.Context(), token, id, input.ClientTurnID, input.Content)
		supportRespond(w, map[string]any{"message": message}, err)
	}))
	mux.HandleFunc("POST /api/assistant/support/{id}/accept", withID(func(w http.ResponseWriter, r *http.Request, token string, id int64) {
		var input struct{}
		if !body(w, r, &input) {
			return
		}
		request, err := support.WebAcceptSupport(r.Context(), token, id)
		supportRespond(w, map[string]any{"request": request}, err)
	}))
	mux.HandleFunc("POST /api/assistant/support/{id}/close", withID(func(w http.ResponseWriter, r *http.Request, token string, id int64) {
		var input struct {
			Cancel bool `json:"cancel"`
		}
		if !body(w, r, &input) {
			return
		}
		request, err := support.WebCloseSupport(r.Context(), token, id, input.Cancel)
		supportRespond(w, map[string]any{"request": request}, err)
	}))
}

// supportRespond keeps the original human-support error codes.
func supportRespond(w http.ResponseWriter, value any, err error) {
	code := http.StatusOK
	body := map[string]any{"success": true, "message": "", "data": value}
	if err != nil {
		status, label := http.StatusServiceUnavailable, "ASSISTANT_SUPPORT_UNAVAILABLE"
		switch {
		case errors.Is(err, assistant_services.ErrSupportIneligible):
			status, label = http.StatusForbidden, "ASSISTANT_SUPPORT_RECHARGE_REQUIRED"
		case errors.Is(err, access.ErrUnauthorized):
			status, label = http.StatusUnauthorized, "ASSISTANT_SESSION_REQUIRED"
		case errors.Is(err, access.ErrForbidden):
			status, label = http.StatusForbidden, "ASSISTANT_SUPPORT_FORBIDDEN"
		case errors.Is(err, access.ErrNotFound):
			status, label = http.StatusNotFound, "ASSISTANT_SUPPORT_NOT_FOUND"
		case errors.Is(err, access.ErrConflict):
			status, label = http.StatusConflict, "ASSISTANT_SUPPORT_CONFLICT"
		case errors.Is(err, access.ErrInvalid):
			status, label = http.StatusUnprocessableEntity, "ASSISTANT_SUPPORT_INVALID"
		case errors.Is(err, access.ErrLimit):
			status, label = http.StatusTooManyRequests, "ASSISTANT_LIMIT_REACHED"
		}
		code = status
		body = map[string]any{"success": false, "code": label, "message": "Human support request could not be completed."}
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(body)
}
