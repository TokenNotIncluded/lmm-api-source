// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package herosms

import (
	"context"
	"net/http"
	"strconv"
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

// Only these public settings cross the browser boundary. Credentials and
// provider diagnostics remain in Core, even if its RPC adds fields later.
type settingsProjection struct {
	Revision                string `json:"revision"`
	Enabled                 bool   `json:"enabled"`
	SMS                     bool   `json:"sms_enabled"`
	Email                   bool   `json:"email_enabled"`
	APIKeyConfigured        bool   `json:"api_key_configured"`
	CredentialManagedByFile bool   `json:"credential_managed_by_file"`
	PendingWork             bool   `json:"pending_work"`
	PurchasesAvailable      bool   `json:"purchases_available"`
	Currency                string `json:"currency"`
	CurrencyCode            int    `json:"currency_code"`
	Multiplier              string `json:"price_multiplier"`
}

func (s *Service) settingsSnapshot(ctx context.Context, user string) (settingsProjection, error) {
	var current struct {
		Revision           int64  `json:"revision"`
		Enabled            bool   `json:"enabled"`
		SMS                bool   `json:"sms_enabled"`
		Email              bool   `json:"email_enabled"`
		APIKeyConfigured   bool   `json:"api_key_configured"`
		PendingWork        bool   `json:"pending_work"`
		PurchasesAvailable bool   `json:"purchases_available"`
		Multiplier         string `json:"price_multiplier"`
	}
	if err := s.Core.Call(ctx, "settings", user, struct{}{}, &current); err != nil {
		return settingsProjection{}, err
	}
	return settingsProjection{
		Revision: strconv.FormatInt(current.Revision, 10), Enabled: current.Enabled,
		SMS: current.SMS, Email: current.Email, APIKeyConfigured: current.APIKeyConfigured,
		CredentialManagedByFile: true, PendingWork: current.PendingWork,
		PurchasesAvailable: current.PurchasesAvailable,
		Currency:           "USD", CurrencyCode: 840, Multiplier: current.Multiplier,
	}, nil
}

func (s *Service) settings(w http.ResponseWriter, r *http.Request, user string) {
	var out any
	var err error
	switch {
	case r.Method == http.MethodGet && r.URL.Path == "/api/option/hero-sms":
		out, err = s.settingsSnapshot(r.Context(), user)
	case r.Method == http.MethodPut && r.URL.Path == "/api/option/hero-sms":
		var in struct {
			ExpectedRevision string `json:"expected_revision"`
			Enabled          *bool  `json:"enabled"`
			SMS              *bool  `json:"sms_enabled"`
			Email            *bool  `json:"email_enabled"`
			Multiplier       string `json:"price_multiplier"`
			Key              string `json:"api_key"`
		}
		if err = decode(r, &in); err == nil {
			if in.Enabled == nil || in.SMS == nil || in.Email == nil {
				respond(w, nil, ErrInvalid)
				return
			}
			if in.Key != "" {
				_, err = s.settingsSnapshot(r.Context(), user)
				if err == nil {
					err = &Failure{409, "CREDENTIAL_MANAGED_BY_FILE"}
				}
				respond(w, nil, err)
				return
			}
			revision, parseErr := strconv.ParseInt(in.ExpectedRevision, 10, 64)
			if parseErr != nil || revision < 0 || strconv.FormatInt(revision, 10) != in.ExpectedRevision {
				respond(w, nil, ErrInvalid)
				return
			}
			err = s.Core.Call(r.Context(), "save-policy", user, map[string]any{"revision": revision, "enabled": in.Enabled, "sms_enabled": in.SMS, "email_enabled": in.Email, "price_multiplier": in.Multiplier}, &out)
			if err == nil {
				out, err = s.settingsSnapshot(r.Context(), user)
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
				var result struct {
					OK bool `json:"ok"`
				}
				err = s.Core.Call(r.Context(), "test", user, struct{}{}, &result)
				out = result
			}
		}
	case r.Method == http.MethodDelete && r.URL.Path == "/api/option/hero-sms/key":
		_, err = s.settingsSnapshot(r.Context(), user)
		if err == nil {
			err = &Failure{409, "CREDENTIAL_MANAGED_BY_FILE"}
		}
	default:
		err = &Failure{404, "NOT_FOUND"}
	}
	respond(w, out, err)
}
