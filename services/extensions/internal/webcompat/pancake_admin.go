package webcompat

import (
	"context"
	"encoding/json"
	"io"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// PancakeAdminRequest preserves the existing Web request fields. Credentials
// are request bodies only and must never be returned in provider errors.
type PancakeAdminRequest struct {
	MerchantID string `json:"merchant_id"`
	PrivateKey string `json:"private_key"`
	ReturnURL  string `json:"return_url"`
	StoreID    string `json:"store_id"`
	ProductID  string `json:"product_id"`
}

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
			Respond(w, value, err)
		})
	}
}
