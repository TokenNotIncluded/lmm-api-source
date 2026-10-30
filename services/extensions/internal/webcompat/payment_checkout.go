// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"net/http"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Browser checkout derives the personal payer inside Core. A browser never
// supplies a storage account ID, credit conversion, provider URL or merchant.
type PaymentCheckoutService interface {
	Catalog() []payments.PublicChannel
	Preview(context.Context, string, payments.PrepareRequest) (payments.Intent, error)
	Create(context.Context, string, payments.PrepareRequest) (payments.Order, error)
	Get(context.Context, string, string) (payments.Order, error)
}

type paymentCheckoutInput struct {
	ChannelID           string `json:"channel_id"`
	AmountMinor         string `json:"amount_minor"`
	Currency            string `json:"currency"`
	RequestKey          string `json:"request_key"`
	OrderID             string `json:"order_id"`
	ExpectedCreditUnits string `json:"expected_credit_units"`
}

type paymentQuoteView struct {
	OrderID     string `json:"order_id"`
	ChannelID   string `json:"channel_id"`
	AccountID   string `json:"account_id"`
	Provider    string `json:"provider"`
	Environment string `json:"environment"`
	Currency    string `json:"currency"`
	AmountMinor string `json:"amount_minor"`
	CreditUnits string `json:"credit_units"`
	CreditUnit  string `json:"credit_unit"`
	RequestKey  string `json:"request_key"`
}

type paymentCheckoutView struct {
	OrderID             string `json:"order_id"`
	State               string `json:"state"`
	Currency            string `json:"currency"`
	AmountMinor         string `json:"amount_minor"`
	CreditUnits         string `json:"credit_units"`
	CreditUnit          string `json:"credit_unit"`
	CheckoutURL         string `json:"checkout_url,omitempty"`
	CheckoutMethod      string `json:"checkout_method"`
	ProviderPaid        bool   `json:"provider_paid"`
	CoreCreditConfirmed bool   `json:"core_credit_confirmed"`
}

func paymentDecimalID(value string) (int64, bool) {
	n, err := strconv.ParseInt(value, 10, 64)
	return n, err == nil && n > 0 && strconv.FormatInt(n, 10) == value
}

func checkoutView(o payments.Order) paymentCheckoutView {
	method := o.Checkout.Method
	if method == "" {
		method = http.MethodGet
	}
	return paymentCheckoutView{OrderID: o.ID, State: o.State(), Currency: o.Intent.Currency,
		AmountMinor: strconv.FormatInt(o.Intent.AmountMinor, 10), CreditUnits: strconv.FormatInt(o.Intent.CreditUnits, 10), CreditUnit: o.Intent.CreditUnit,
		CheckoutURL: o.Checkout.URL, CheckoutMethod: method, ProviderPaid: o.ProviderPaid, CoreCreditConfirmed: o.Credited()}
}

func checkoutData(w http.ResponseWriter, code int, value any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(map[string]any{"success": true, "data": value})
}

func RegisterPaymentCheckout(mux *http.ServeMux, auth *Authenticator, service PaymentCheckoutService) {
	ready := func(w http.ResponseWriter, r *http.Request) (string, int64, bool) {
		self, err := auth.Verify(r.Context(), r)
		if err != nil {
			writePaymentError(w, err)
			return "", 0, false
		}
		if store.Missing(service) {
			writePayment(w, http.StatusServiceUnavailable, "PAYMENT_DISABLED", "Payment checkout is not configured.")
			return "", 0, false
		}
		return strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), self.GetUserId(), true
	}
	mux.HandleFunc("GET /api/user/payments/channels", func(w http.ResponseWriter, r *http.Request) {
		if _, _, ok := ready(w, r); !ok {
			return
		}
		type channelView struct {
			ID               string `json:"id"`
			Provider         string `json:"provider"`
			Environment      string `json:"environment"`
			Currency         string `json:"currency"`
			PaymentType      string `json:"payment_type,omitempty"`
			ProductID        string `json:"product_id,omitempty"`
			FixedAmountMinor string `json:"fixed_amount_minor,omitempty"`
		}
		channels := make([]channelView, 0)
		for _, c := range service.Catalog() {
			view := channelView{ID: strconv.FormatInt(c.ID, 10), Provider: c.Provider, Environment: c.Environment, Currency: c.Currency, PaymentType: c.PaymentType, ProductID: c.ProductID}
			if c.FixedAmountMinor > 0 {
				view.FixedAmountMinor = strconv.FormatInt(c.FixedAmountMinor, 10)
			}
			channels = append(channels, view)
		}
		checkoutData(w, http.StatusOK, map[string]any{"channels": channels, "available": len(channels) > 0})
	})
	write := func(confirm bool) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			token, userID, ok := ready(w, r)
			if !ok {
				return
			}
			var input paymentCheckoutInput
			fields := []string{"channel_id", "amount_minor", "currency", "request_key"}
			if confirm {
				fields = append(fields, "order_id", "expected_credit_units")
			}
			if r.URL.RawQuery != "" || supportDecode(w, r, 4096, &input, fields...) != nil {
				writePayment(w, http.StatusBadRequest, "PAYMENT_INVALID", "Invalid payment request.")
				return
			}
			channelID, okChannel := paymentDecimalID(input.ChannelID)
			amount, okAmount := paymentDecimalID(input.AmountMinor)
			if !okChannel || !okAmount || !validPaymentCurrency(input.Currency) {
				writePayment(w, http.StatusBadRequest, "PAYMENT_INVALID", "Invalid payment amount or channel.")
				return
			}
			req := payments.PrepareRequest{Key: input.RequestKey, ChannelID: channelID, Currency: input.Currency, AmountMinor: amount}
			intent, err := service.Preview(r.Context(), token, req)
			if err != nil {
				writePaymentError(w, err)
				return
			}
			if intent.CreatedBy != userID {
				writePayment(w, http.StatusForbidden, "PAYMENT_ACCESS_DENIED", "Payment account could not be authorized.")
				return
			}
			quote := paymentQuoteView{OrderID: "top_" + strconv.FormatInt(intent.ID, 10), ChannelID: input.ChannelID, AccountID: strconv.FormatInt(intent.AccountID, 10), Provider: intent.Provider, Environment: intent.Environment, Currency: intent.Currency, AmountMinor: strconv.FormatInt(intent.AmountMinor, 10), CreditUnits: strconv.FormatInt(intent.CreditUnits, 10), CreditUnit: intent.CreditUnit, RequestKey: intent.Key}
			if !confirm {
				checkoutData(w, http.StatusOK, quote)
				return
			}
			if input.OrderID != quote.OrderID || input.ExpectedCreditUnits != quote.CreditUnits {
				writePayment(w, http.StatusConflict, "PAYMENT_QUOTE_CHANGED", "Review the original quote before confirming payment.")
				return
			}
			req.AccountID = intent.AccountID
			order, err := service.Create(r.Context(), token, req)
			if err != nil {
				writePaymentError(w, err)
				return
			}
			checkoutData(w, http.StatusAccepted, checkoutView(order))
		}
	}
	mux.HandleFunc("POST /api/user/payments/quote", write(false))
	mux.HandleFunc("POST /api/user/payments/checkout", write(true))
	mux.HandleFunc("GET /api/user/payments/checkout/{id}", func(w http.ResponseWriter, r *http.Request) {
		token, _, ok := ready(w, r)
		if !ok {
			return
		}
		id := r.PathValue("id")
		if !validPaymentOrderID(id) || r.URL.RawQuery != "" {
			writePaymentError(w, payments.ErrNotFound)
			return
		}
		order, err := service.Get(r.Context(), token, id)
		if err != nil {
			writePaymentError(w, err)
			return
		}
		checkoutData(w, http.StatusOK, checkoutView(order))
	})
}
