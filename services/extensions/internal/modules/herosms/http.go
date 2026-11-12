// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package herosms

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"sort"
	"strconv"
	"strings"
)

type Service struct{ Core *Core }

func (s *Service) Name() string          { return "hero-sms" }
func (s *Service) Handler() http.Handler { return s }
func Owns(path string) bool {
	return strings.HasPrefix(path, "/api/hero-sms/") || path == "/api/option/hero-sms" || strings.HasPrefix(path, "/api/option/hero-sms/")
}
func respond(w http.ResponseWriter, data any, err error) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	if err != nil {
		code, message := status(err)
		w.WriteHeader(code)
		_ = json.NewEncoder(w).Encode(map[string]any{"success": false, "code": message, "message": message, "data": nil})
		return
	}
	_ = json.NewEncoder(w).Encode(map[string]any{"success": true, "data": data})
}
func decode(r *http.Request, out any) error {
	d := json.NewDecoder(io.LimitReader(r.Body, (64<<10)+1))
	d.DisallowUnknownFields()
	if d.Decode(out) != nil || d.Decode(new(any)) != io.EOF {
		return ErrInvalid
	}
	return nil
}
func queryInt(r *http.Request, key string, def int64) int64 {
	v := r.URL.Query().Get(key)
	if v == "" {
		return def
	}
	n, e := strconv.ParseInt(v, 10, 64)
	if e != nil {
		return -1
	}
	return n
}
func randomKey() string {
	var b [32]byte
	if _, err := rand.Read(b[:]); err != nil {
		return ""
	}
	return hex.EncodeToString(b[:])
}
func (s *Service) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	user := r.Header.Get("X-LMM-User-Credential")
	if user == "" || len(r.Header.Values("X-LMM-User-Credential")) != 1 {
		respond(w, nil, &Failure{403, "FORBIDDEN"})
		return
	}
	path := strings.TrimPrefix(r.URL.Path, "/api/hero-sms")
	if strings.HasPrefix(r.URL.Path, "/api/option/hero-sms") {
		s.settings(w, r, user)
		return
	}
	var out any
	var err error
	switch {
	case r.Method == http.MethodGet && (path == "/sms/countries" || path == "/sms/services" || path == "/sms/operators"):
		out, err = s.reference(r, user, strings.TrimPrefix(path, "/sms/"))
	case r.Method == http.MethodGet && path == "/sms/offer":
		var q Quote
		selection := Selection{Kind: "sms", Country: queryInt(r, "country", -1), Service: r.URL.Query().Get("service"), Operator: r.URL.Query().Get("operator")}
		err = s.Core.Call(r.Context(), "quote", user, map[string]any{"request_key": randomKey(), "selection": selection, "max_customer_price_usd": r.URL.Query().Get("max_price_usd")}, &q)
		out = offer(q)
	case r.Method == http.MethodPost && path == "/sms/orders":
		var input struct {
			OfferID string `json:"offer_id"`
		}
		if err = decode(r, &input); err == nil {
			out, err = s.purchase(r, user, input.OfferID, 1, "", "sms")
		}
	case r.Method == http.MethodGet && (path == "/sms/orders" || path == "/sms/orders/current-list"):
		out, err = s.list(r, user, "sms", path == "/sms/orders/current-list")
	case r.Method == http.MethodDelete && (path == "/sms/history" || strings.HasPrefix(path, "/sms/history/")):
		id := strings.TrimPrefix(path, "/sms/history")
		id = strings.TrimPrefix(id, "/")
		var result struct {
			Count int64 `json:"hidden_count"`
		}
		err = s.Core.Call(r.Context(), "hide", user, map[string]any{"id": id}, &result)
		out = map[string]any{"hidden_count": result.Count, "hidden": err == nil}
	case strings.HasPrefix(path, "/sms/orders/"):
		out, err = s.order(r, user, strings.TrimPrefix(path, "/sms/orders/"), "sms")
	case r.Method == http.MethodGet && path == "/email/products":
		var page struct {
			Items []Quote `json:"items"`
			Page  int64   `json:"page"`
			Size  int64   `json:"size"`
			Total int64   `json:"total"`
		}
		err = s.Core.Call(r.Context(), "email-products", user, map[string]any{"site": r.URL.Query().Get("site"), "page": queryInt(r, "page", 1), "size": queryInt(r, "size", 100)}, &page)
		items := make([]any, 0, len(page.Items))
		for _, q := range page.Items {
			items = append(items, product(q))
		}
		out = map[string]any{"items": items, "page": page.Page, "size": page.Size, "total": page.Total}
	case r.Method == http.MethodPost && path == "/email/activations":
		var input struct {
			DomainID string `json:"domain_id"`
			Quantity int    `json:"quantity"`
		}
		if err = decode(r, &input); err == nil {
			out, err = s.purchase(r, user, input.DomainID, input.Quantity, "", "email")
		}
	case r.Method == http.MethodGet && path == "/email/activations":
		out, err = s.list(r, user, "email", false)
	case r.Method == http.MethodGet && path == "/email/activations/current":
		var p Page
		err = s.Core.Call(r.Context(), "list", user, map[string]any{"kind": "email", "active": true, "page": 1, "size": 1}, &p)
		if len(p.Items) > 0 {
			out = email(p.Items[0])
		}
	case strings.HasPrefix(path, "/email/activations/"):
		out, err = s.order(r, user, strings.TrimPrefix(path, "/email/activations/"), "email")
	default:
		err = &Failure{404, "NOT_FOUND"}
	}
	respond(w, out, err)
}
func (s *Service) purchase(r *http.Request, user, quote string, quantity int, reorder, kind string) (any, error) {
	key := r.Header.Get("Idempotency-Key")
	if len(r.Header.Values("Idempotency-Key")) != 1 || len(key) < 16 || len(key) > 128 {
		return nil, ErrInvalid
	}
	var plan Plan
	if err := s.Core.Call(r.Context(), "plan", user, map[string]any{"request_key": key, "quote_id": quote, "quantity": quantity, "reorder_id": reorder}, &plan); err != nil {
		return nil, err
	}
	items := make([]any, 0, len(plan.Orders))
	var last Receipt
	batchState := "active"
	failed := 0
	var reserved int64
	var first error
	for _, id := range plan.Orders {
		var receipt Receipt
		err := s.Core.Call(r.Context(), "buy", user, map[string]any{"id": id, "quote_id": plan.QuoteID, "reorder_id": plan.ReorderID}, &receipt)
		if err != nil {
			first = err
			break
		}
		last = receipt
		reserved += receipt.Charge
		if receipt.Quote.Selection.Kind != kind {
			return nil, ErrUnavailable
		}
		state := viewState(receipt)
		if state == "purchase_unknown" || state == "pending_provider" {
			batchState = "purchase_unknown"
		} else if state == "failed" {
			failed++
		}
		if kind == "email" {
			items = append(items, email(receipt))
		}
	}
	if first != nil && (kind == "sms" || len(items) == 0) {
		return nil, first
	}
	if kind == "sms" {
		var balance struct {
			Quota int64 `json:"quota"`
		}
		if err := s.Core.Call(r.Context(), "balance", user, struct{}{}, &balance); err != nil {
			return nil, err
		}
		return map[string]any{"order": sms(last), "quota": balance.Quota}, nil
	}
	if first != nil || (failed > 0 && failed < len(plan.Orders)) {
		batchState = "partial"
	} else if failed == len(plan.Orders) {
		batchState = "failed"
	}
	errorCode := ""
	if first != nil {
		_, errorCode = status(first)
	}
	return map[string]any{"order": map[string]any{"id": plan.ID, "status": batchState, "quantity": len(plan.Orders), "received_count": len(items), "failed_count": failed, "last_error_code": errorCode, "charge_quota": reserved, "created_at": last.Created, "updated_at": last.Updated}, "activations": items}, nil
}
func (s *Service) order(r *http.Request, user, tail, kind string) (any, error) {
	parts := strings.Split(tail, "/")
	if len(parts) > 2 || parts[0] == "" {
		return nil, ErrInvalid
	}
	action := "query"
	input := map[string]any{"id": parts[0]}
	if kind == "sms" {
		action = "poll"
	}
	if len(parts) == 2 {
		switch parts[1] {
		case "refresh":
			action = "poll"
		case "cancel":
			action = "cancel"
		case "complaints":
			if kind != "sms" || r.Method != http.MethodPost {
				return nil, ErrInvalid
			}
			var body struct {
				Reason string `json:"reason"`
			}
			if err := decode(r, &body); err != nil {
				return nil, err
			}
			input["reason"] = body.Reason
			action = "complaint"
		case "reorder":
			if kind != "email" || r.Method != http.MethodPost {
				return nil, ErrInvalid
			}
			var input struct {
				DomainID string `json:"domain_id"`
			}
			if err := decode(r, &input); err != nil {
				return nil, err
			}
			return s.purchase(r, user, input.DomainID, 1, parts[0], kind)
		default:
			return nil, &Failure{400, "UNSUPPORTED_ACTION"}
		}
		if r.Method != http.MethodPost {
			return nil, &Failure{405, "METHOD_NOT_ALLOWED"}
		}
	} else if r.Method != http.MethodGet {
		return nil, &Failure{405, "METHOD_NOT_ALLOWED"}
	}
	var out Receipt
	if err := s.Core.Call(r.Context(), action, user, input, &out); err != nil {
		return nil, err
	}
	if out.Quote.Selection.Kind != kind {
		return nil, &Failure{404, "NOT_FOUND"}
	}
	if kind == "email" {
		return map[string]any{"activation": email(out)}, nil
	}
	var balance struct {
		Quota int64 `json:"quota"`
	}
	if err := s.Core.Call(r.Context(), "balance", user, struct{}{}, &balance); err != nil {
		return nil, err
	}
	return map[string]any{"order": sms(out), "quota": balance.Quota}, nil
}
func (s *Service) list(r *http.Request, user, kind string, active bool) (any, error) {
	state := r.URL.Query().Get("status")
	switch state {
	case "active", "pending_provider", "purchase_unknown":
		state = "reserved"
	case "failed":
		state = "rejected"
	}
	var page Page
	if err := s.Core.Call(r.Context(), "list", user, map[string]any{"kind": kind, "active": active, "page": queryInt(r, "page", 1), "size": queryInt(r, "size", 20), "status": state}, &page); err != nil {
		return nil, err
	}
	items := make([]any, 0, len(page.Items))
	for _, v := range page.Items {
		if kind == "sms" {
			items = append(items, sms(v))
		} else {
			items = append(items, email(v))
		}
	}
	return map[string]any{"items": items, "page": page.Page, "size": page.Size, "total": page.Total}, nil
}
func (s *Service) reference(r *http.Request, user, kind string) (any, error) {
	var raw map[string]json.RawMessage
	if err := s.Core.Call(r.Context(), "reference", user, map[string]any{"kind": kind, "country": queryInt(r, "country", 0)}, &raw); err != nil {
		return nil, err
	}
	items := []any{}
	switch kind {
	case "countries":
		keys := make([]string, 0, len(raw))
		for k := range raw {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			var v struct {
				ID      int64           `json:"id"`
				Eng     string          `json:"eng"`
				Chn     string          `json:"chn"`
				Rus     string          `json:"rus"`
				Visible json.RawMessage `json:"visible"`
			}
			if json.Unmarshal(raw[k], &v) != nil {
				continue
			}
			id, e := strconv.ParseInt(k, 10, 64)
			if e != nil {
				continue
			}
			if string(v.Visible) == "0" || string(v.Visible) == "false" {
				continue
			}
			name := v.Eng
			if name == "" {
				name = v.Rus
			}
			items = append(items, map[string]any{"id": id, "name": name, "english_name": v.Eng, "chinese_name": v.Chn, "popularity": 0})
		}
		return items, nil
	case "services":
		var services []struct {
			Code string `json:"code"`
			Name string `json:"name"`
		}
		if json.Unmarshal(raw["services"], &services) != nil {
			return nil, ErrUnavailable
		}
		for _, v := range services {
			items = append(items, map[string]any{"code": v.Code, "name": v.Name, "popularity": 0})
		}
		return items, nil
	case "operators":
		var all map[string][]string
		if json.Unmarshal(raw["countryOperators"], &all) != nil {
			return nil, ErrUnavailable
		}
		out := all[strconv.FormatInt(queryInt(r, "country", 0), 10)]
		if out == nil {
			out = []string{}
		}
		return out, nil
	}
	return nil, ErrInvalid
}
