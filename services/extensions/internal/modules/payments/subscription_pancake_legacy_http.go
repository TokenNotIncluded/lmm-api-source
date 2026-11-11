package payments

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"sort"
	"strings"
)

// The app mounts this at the original go-v0.2.118 webhook URL with the SAME
// body, deadline, rate, header and shared-host admission guards as new callbacks.
func (s *PancakeSubscriptions) LegacyWebhookHandler(wallet *Service) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		raw, _, err := callbackBytes(r, "X-Waffo-Signature")
		if err != nil {
			paymentError(w, err)
			return
		}
		var event struct {
			EventType string `json:"eventType"`
			Mode      string `json:"mode"`
			Store     string `json:"storeId"`
			Data      struct {
				Reference string `json:"orderMerchantExternalId"`
				Currency  string `json:"currency"`
			} `json:"data"`
		}
		if json.Unmarshal(raw, &event) != nil || event.Mode != r.PathValue("environment") {
			paymentError(w, ErrInvalid)
			return
		}
		r.Body = http.NoBody
		request := func() *http.Request {
			copy := r.Clone(r.Context())
			copy.Body = http.NoBody
			copy.Body = io.NopCloser(bytes.NewReader(raw))
			return copy
		}
		if strings.HasPrefix(event.Data.Reference, "WAFFO_PANCAKE_SUB-") || strings.HasPrefix(event.Data.Reference, "sub_") || strings.HasPrefix(event.EventType, "subscription.") {
			if _, err = s.Accept(r.Context(), 0, request()); err != nil {
				paymentError(w, err)
				return
			}
		} else {
			if wallet == nil {
				paymentError(w, ErrUnavailable)
				return
			}
			environment := "test"
			if event.Mode == "prod" {
				environment = "live"
			}
			var ids []int64
			for id, a := range s.adapters {
				if a.cfg.Channel.Environment == environment && a.cfg.StoreID == event.Store && a.cfg.Channel.Currency == event.Data.Currency {
					ids = append(ids, id)
				}
			}
			sort.Slice(ids, func(i, j int) bool { return ids[i] < ids[j] })
			// Legacy originals are resolved by Core's reviewed channel/reference basis;
			// never guess wallet account/credits from signed provider metadata.
			err = ErrNotFound
			if len(ids) == 0 {
				// An unknown currency/store has no reviewed channel binding. Keep
				// its exact evidence unbound; channel zero cannot admit wallet credit.
				err = s.receiveWallet(r.Context(), wallet, 0, request())
			}
			for _, id := range ids {
				err = wallet.Receive(r.Context(), id, request())
				if err == nil || !errors.Is(err, ErrNotFound) {
					break
				}
			}
			if err != nil {
				paymentError(w, err)
				return
			}
		}
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		_, _ = w.Write([]byte("OK"))
	})
}
