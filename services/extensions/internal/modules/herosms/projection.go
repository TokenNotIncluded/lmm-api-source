// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package herosms

import (
	"net/http"
	"time"
)

func viewState(r Receipt) string {
	switch r.State {
	case "rejected":
		return "failed"
	case "reserved":
		if !r.Dispatched {
			return "pending_provider"
		}
		if r.ProviderID == "" || !r.Usable {
			return "purchase_unknown"
		}
		return "active"
	default:
		return r.State
	}
}
func offer(q Quote) map[string]any {
	return map[string]any{"id": q.ID, "country_id": q.Selection.Country, "service": q.Selection.Service, "operator": q.Selection.Operator, "inventory": q.Inventory, "customer_price_usd": q.Price, "charge_quota": q.Charge, "pricing_schema_version": 2, "pricing_currency": "USD", "pricing_available": true, "expires_at": q.Expires, "bid": q.Bid, "tiers": q.Tiers}
}
func product(q Quote) map[string]any {
	return map[string]any{"id": q.ID, "domain": q.Selection.Domain, "site": q.Selection.Site, "count": q.Inventory, "available": q.Inventory > 0, "customer_price_usd": q.Price, "charge_quota": q.Charge, "pricing_schema_version": 2, "pricing_currency": "USD", "pricing_available": true, "expires_at": q.Expires}
}
func sms(r Receipt) map[string]any {
	a := r.Activation
	if a == nil || !r.Usable {
		a = &Activation{}
	}
	code := ""
	message := ""
	if viewState(r) == "purchase_unknown" {
		code = "PURCHASE_UNKNOWN"
		message = "Supplier outcome is unknown. Funds remain reserved; contact an administrator and do not place another order."
	}
	return map[string]any{"id": r.ID, "country_id": r.Quote.Selection.Country, "service": r.Quote.Selection.Service, "operator": r.Quote.Selection.Operator, "status": viewState(r), "customer_price_usd": r.Quote.Price, "charge_quota": r.Charge, "refunded_quota": r.Refunded, "provider_id": r.ProviderID, "can_cancel": r.State == "reserved" && (r.ProviderID != "" || !r.Dispatched), "can_complain": viewState(r) == "active" && r.ComplaintType == "" && time.Now().Unix() >= r.Created+120, "complaint_type": r.ComplaintType, "complaint_status": r.ComplaintStatus, "complaint_submitted_at": r.ComplaintSubmitted, "phone_number": a.Phone, "code": a.Code, "message": a.Message, "last_error_code": code, "last_error_message": message, "created_at": r.Created, "updated_at": r.Updated, "pricing_schema_version": 2, "pricing_currency": "USD", "pricing_available": true}
}
func email(r Receipt) map[string]any {
	a := r.Activation
	if a == nil || !r.Usable {
		a = &Activation{}
	}
	return map[string]any{"id": r.ID, "order_id": r.ID, "domain_id": r.QuoteID, "email": a.Email, "code": a.Code, "message": a.Message, "site": r.Quote.Selection.Site, "domain": r.Quote.Selection.Domain, "status": viewState(r), "charge_quota": r.Charge, "refunded_quota": r.Refunded, "cancel_reason": "", "created_at": r.Created, "updated_at": r.Updated}
}
func (s *Service) settings(w http.ResponseWriter, r *http.Request, user string) {
	var out any
	var err error
	switch {
	case r.Method == http.MethodGet && r.URL.Path == "/api/option/hero-sms":
		err = s.Core.Call(r.Context(), "settings", user, struct{}{}, &out)
	case r.Method == http.MethodPut && r.URL.Path == "/api/option/hero-sms":
		var in struct {
			Enabled    *bool  `json:"enabled"`
			SMS        *bool  `json:"sms_enabled"`
			Email      *bool  `json:"email_enabled"`
			Multiplier string `json:"price_multiplier"`
			Key        string `json:"api_key"`
		}
		if err = decode(r, &in); err == nil {
			if in.Enabled == nil || in.SMS == nil || in.Email == nil {
				respond(w, nil, ErrInvalid)
				return
			}
			var current struct {
				Revision int64 `json:"revision"`
			}
			err = s.Core.Call(r.Context(), "settings", user, struct{}{}, &current)
			if err == nil && in.Key != "" {
				err = &Failure{409, "CREDENTIAL_MANAGED_BY_FILE"}
			}
			if err == nil {
				err = s.Core.Call(r.Context(), "save-policy", user, map[string]any{"revision": current.Revision, "enabled": in.Enabled, "sms_enabled": in.SMS, "email_enabled": in.Email, "price_multiplier": in.Multiplier}, &out)
			}
			if err == nil {
				err = s.Core.Call(r.Context(), "settings", user, struct{}{}, &out)
			}
		}
	case r.Method == http.MethodPost && r.URL.Path == "/api/option/hero-sms/test":
		var in struct {
			Key string `json:"api_key"`
		}
		if err = decode(r, &in); err == nil {
			if in.Key != "" {
				err = &Failure{409, "CREDENTIAL_MANAGED_BY_FILE"}
			} else {
				err = s.Core.Call(r.Context(), "test", user, struct{}{}, &out)
			}
		}
	case r.Method == http.MethodDelete && r.URL.Path == "/api/option/hero-sms/key":
		err = s.Core.Call(r.Context(), "settings", user, struct{}{}, &out)
		if err == nil {
			err = &Failure{409, "CREDENTIAL_MANAGED_BY_FILE"}
		}
	default:
		err = &Failure{404, "NOT_FOUND"}
	}
	respond(w, out, err)
}
