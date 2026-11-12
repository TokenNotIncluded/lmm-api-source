package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// PancakeAdminRequest preserves the existing Web request fields. Credentials
// are request bodies only and must never be returned in provider errors.
type PancakeAdminRequest = payments.PancakeAdminRequest

type PancakeAdmin interface {
	Catalog(context.Context, PancakeAdminRequest) (any, error)
	Pair(context.Context, PancakeAdminRequest) (any, error)
	Save(context.Context, PancakeAdminRequest) (any, error)
}

// RegisterPancakeAdmin requires a live personal owner session for every action,
// including catalogue probes that exercise merchant credentials.
func RegisterPancakeAdmin(mux *http.ServeMux, auth *Authenticator, admin PancakeAdmin) {
	if store.Missing(admin) {
		return
	}
	for action, invoke := range map[string]func(context.Context, PancakeAdminRequest) (any, error){
		"catalog": admin.Catalog, "pair": admin.Pair, "save": admin.Save,
	} {
		mux.HandleFunc("POST /api/option/waffo-pancake/"+action, func(w http.ResponseWriter, r *http.Request) {
			self, err := auth.Verify(r.Context(), r)
			if err != nil {
				Respond(w, nil, err)
				return
			}
			if self.GetPlatformLevel() != 6 {
				Respond(w, nil, store.ErrForbidden)
				return
			}
			decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 64<<10))
			decoder.DisallowUnknownFields()
			var request PancakeAdminRequest
			if decoder.Decode(&request) != nil || decoder.Decode(new(any)) != io.EOF {
				Respond(w, nil, store.ErrInvalid)
				return
			}
			value, err := invoke(r.Context(), request)
			respondPancakeAdmin(w, value, err)
		})
	}
}

func respondPancakeAdmin(w http.ResponseWriter, value any, err error) {
	if errors.Is(err, payments.ErrPancakePolicy) {
		writePayment(w, http.StatusConflict, "PAYMENT_CORE_POLICY_REQUIRED", "Update the Core payment channel policy before changing this merchant, store or product.")
		return
	}
	var committed *payments.ConfigurationCommittedError
	if errors.As(err, &committed) {
		writePayment(w, http.StatusServiceUnavailable, "PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN", "The running configuration was updated, but crash durability is unknown. Read the current configuration before retrying.")
		return
	}
	if err != nil {
		if partial, ok := value.(*payments.PancakePairResult); ok && partial != nil && partial.OrphanStore {
			w.Header().Set("Content-Type", "application/json; charset=utf-8")
			w.Header().Set("Cache-Control", "no-store")
			w.WriteHeader(http.StatusBadGateway)
			_ = json.NewEncoder(w).Encode(map[string]any{"success": false, "message": "error", "code": "PAYMENT_PAIR_INCOMPLETE", "data": partial})
			return
		}
		writePaymentError(w, err)
		return
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	_ = json.NewEncoder(w).Encode(map[string]any{"success": true, "message": "success", "data": value})
}
