// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/url"
	"path"
	"strconv"
	"strings"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func ioNetRespond(w http.ResponseWriter, value any, err error) {
	w.Header().Set("Cache-Control", "no-store")
	if errors.Is(err, operations.ErrIoNetUpstream) {
		ioNetJSONReply(w, http.StatusBadGateway, map[string]any{"success": false, "code": "IONET_UPSTREAM_UNAVAILABLE", "message": "io.net request unavailable"})
		return
	}
	if op, ok := value.(operations.IoNetOperation); err == nil && ok && op.State != "succeeded" && op.State != "cancelled" {
		status := http.StatusAccepted
		code := "IONET_OPERATION_UNKNOWN"
		if op.State == "rejected" {
			status = http.StatusUnprocessableEntity
			code = "IONET_OPERATION_REJECTED"
		}
		ioNetJSONReply(w, status, map[string]any{"success": false, "code": code, "message": code, "data": op})
		return
	}
	operationsRespond(w, value, err)
}

func registerIoNet(mux *http.ServeMux, auth *Authenticator, s *operations.Service) {
	handle := func(root bool, fn func(http.ResponseWriter, *http.Request, *pb.AuthorizeResponse, *operations.IoNet)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			actor, err := auth.Verify(r.Context(), r)
			if err != nil {
				ioNetRespond(w, nil, err)
				return
			}
			if actor.GetUserId() <= 0 || actor.GetUserId() > operations.MaxID || (actor.GetPlatformLevel() != 5 && actor.GetPlatformLevel() != 6) || (root && actor.GetPlatformLevel() != 6) {
				ioNetRespond(w, nil, store.ErrForbidden)
				return
			}
			if s == nil || s.IoNet == nil {
				ioNetRespond(w, nil, operations.ErrUnavailable)
				return
			}
			for _, name := range []string{"id", "container_id", "key"} {
				v := r.PathValue(name)
				if v != "" && !operations.ValidIoNetID(v) {
					ioNetRespond(w, nil, operations.ErrInvalid)
					return
				}
			}
			fn(w, r, actor, s.IoNet)
		}
	}
	mux.HandleFunc("GET /api/deployments/settings", handle(false, func(w http.ResponseWriter, r *http.Request, _ *pb.AuthorizeResponse, s *operations.IoNet) {
		if r.URL.RawQuery != "" {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		v, e := s.Settings(r.Context())
		ioNetRespond(w, v, e)
	}))
	mux.HandleFunc("PUT /api/deployments/settings", handle(true, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
		var in struct {
			Revision *int64 `json:"revision"`
			Enabled  *bool  `json:"enabled"`
			APIKey   string `json:"api_key"`
			Clear    bool   `json:"clear_api_key"`
		}
		if r.URL.RawQuery != "" || supportDecode(w, r, 8192, &in, "revision", "enabled", "api_key", "clear_api_key") != nil || in.Revision == nil || in.Enabled == nil {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		v, e := s.SaveSettings(r.Context(), a.GetUserId(), *in.Revision, *in.Enabled, in.APIKey, in.Clear)
		ioNetRespond(w, v, e)
	}))
	test := handle(false, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
		var in struct {
			APIKey string `json:"api_key"`
		}
		if r.URL.RawQuery != "" || supportDecode(w, r, 8192, &in, "api_key") != nil {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		if in.APIKey != "" && a.GetPlatformLevel() != 6 {
			ioNetRespond(w, nil, store.ErrForbidden)
			return
		}
		v, e := s.TestConnection(r.Context(), in.APIKey)
		ioNetRespond(w, v, e)
	})
	mux.HandleFunc("POST /api/deployments/settings/test-connection", test)
	mux.HandleFunc("POST /api/deployments/test-connection", test)
	read := func(endpoint func(*http.Request) (string, error), adapt func(any) any) http.HandlerFunc {
		return handle(false, func(w http.ResponseWriter, r *http.Request, _ *pb.AuthorizeResponse, s *operations.IoNet) {
			p, e := endpoint(r)
			if e != nil {
				ioNetRespond(w, nil, e)
				return
			}
			v, e := s.Read(r.Context(), p)
			if e == nil && adapt != nil {
				v = adapt(v)
			}
			ioNetRespond(w, v, e)
		})
	}
	list := handle(false, func(w http.ResponseWriter, r *http.Request, _ *pb.AuthorizeResponse, s *operations.IoNet) {
		endpoint, e := ioNetListEndpoint(r)
		if e != nil {
			ioNetRespond(w, nil, e)
			return
		}
		v, e := s.Read(r.Context(), endpoint)
		if e == nil {
			v, e = ioNetListProjection(v, r)
		}
		ioNetRespond(w, v, e)
	})
	mux.HandleFunc("GET /api/deployments/{$}", list)
	mux.HandleFunc("GET /api/deployments/search", list)
	for route, path := range map[string]string{"hardware-types": "/hardware/max-gpus-per-container", "locations": "/locations"} {
		mux.HandleFunc("GET /api/deployments/"+route, read(func(r *http.Request) (string, error) {
			if r.URL.RawQuery != "" {
				return "", operations.ErrInvalid
			}
			return path, nil
		}, func(v any) any {
			if route == "hardware-types" {
				if object, ok := v.(map[string]any); ok {
					if items, found := object["hardware"].([]any); found {
						mapped := make([]any, 0, len(items))
						for _, row := range items {
							if hw, ok := row.(map[string]any); ok {
								mapped = append(mapped, map[string]any{"id": hw["hardware_id"], "name": hw["hardware_name"], "max_gpus": hw["max_gpus_per_container"], "brand_name": hw["brand_name"], "available_count": hw["available"]})
							}
						}
						return map[string]any{"hardware_types": mapped}
					}
				}
			}
			return v
		}))
	}
	mux.HandleFunc("GET /api/deployments/available-replicas", read(func(r *http.Request) (string, error) {
		q, e := ioNetQuery(r, "hardware_id", "gpu_count")
		if e != nil {
			return "", e
		}
		h, e := ioNetPositive(q, "hardware_id", 0, 100000)
		if e != nil {
			return "", e
		}
		g, e := ioNetPositive(q, "gpu_count", 0, 1024)
		if e != nil {
			return "", e
		}
		return "/available-replicas?hardware_id=" + strconv.Itoa(h) + "&hardware_qty=" + strconv.Itoa(g), nil
	}, func(v any) any {
		if rows, ok := v.([]any); ok {
			mapped := make([]any, 0, len(rows))
			for _, row := range rows {
				if item, ok := row.(map[string]any); ok {
					mapped = append(mapped, map[string]any{"location_id": item["id"], "location_name": item["name"], "available_count": item["available_replicas"]})
				}
			}
			return map[string]any{"replicas": mapped}
		}
		return v
	}))
	mux.HandleFunc("GET /api/deployments/check-name", read(func(r *http.Request) (string, error) {
		q, e := ioNetQuery(r, "name")
		if e != nil || !operations.ValidIoNetName(q.Get("name")) {
			return "", operations.ErrInvalid
		}
		return "/clusters/check_cluster_name_availability?cluster_name=" + url.QueryEscape(q.Get("name")), nil
	}, nil))
	mux.HandleFunc("POST /api/deployments/price-estimation", handle(false, func(w http.ResponseWriter, r *http.Request, _ *pb.AuthorizeResponse, s *operations.IoNet) {
		var in operations.IoNetPriceRequest
		if r.URL.RawQuery != "" || supportDecode(w, r, 16384, &in, "location_ids", "hardware_id", "gpus_per_container", "duration_hours", "replica_count", "currency", "duration_type", "duration_qty", "hardware_qty") != nil {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		endpoint, e := in.Endpoint()
		if e != nil {
			ioNetRespond(w, nil, e)
			return
		}
		v, e := s.Read(r.Context(), endpoint)
		ioNetRespond(w, v, e)
	}))
	mux.HandleFunc("GET /api/deployments/operations", handle(false, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
		if r.URL.RawQuery != "" {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		v, e := s.Pending(r.Context(), a.GetUserId())
		ioNetRespond(w, v, e)
	}))
	mux.HandleFunc("GET /api/deployments/operations/receipts/{key}", handle(false, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
		if r.URL.RawQuery != "" {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		v, e := s.Operation(r.Context(), a.GetUserId(), r.PathValue("key"))
		ioNetRespond(w, v, e)
	}))
	mux.HandleFunc("POST /api/deployments/operations/receipts/{key}/recover", handle(true, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
		if r.URL.RawQuery != "" || supportDecode(w, r, 64, &struct{}{}) != nil {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		v, e := s.Recover(r.Context(), a.GetUserId(), r.PathValue("key"))
		ioNetRespond(w, v, e)
	}))

	mux.HandleFunc("POST /api/deployments/operations/receipts/{key}/cancel", handle(true, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
		if r.URL.RawQuery != "" || supportDecode(w, r, 64, &struct{}{}) != nil {
			ioNetRespond(w, nil, operations.ErrInvalid)
			return
		}
		v, e := s.CancelMissing(r.Context(), a.GetUserId(), r.PathValue("key"))
		ioNetRespond(w, v, e)
	}))
	mux.HandleFunc("GET /api/deployments/{id}", read(func(r *http.Request) (string, error) {
		if r.URL.RawQuery != "" {
			return "", operations.ErrInvalid
		}
		return "/deployment/" + r.PathValue("id"), nil
	}, ioNetDeploymentProjection))
	mux.HandleFunc("GET /api/deployments/{id}/containers", read(func(r *http.Request) (string, error) {
		if r.URL.RawQuery != "" {
			return "", operations.ErrInvalid
		}
		return "/deployment/" + r.PathValue("id") + "/containers", nil
	}, func(v any) any {
		if obj, ok := v.(map[string]any); ok {
			if workers, found := obj["workers"]; found {
				return map[string]any{"containers": workers, "total": obj["total"]}
			}
		}
		return v
	}))
	mux.HandleFunc("GET /api/deployments/{id}/containers/{container_id}", read(func(r *http.Request) (string, error) {
		if r.URL.RawQuery != "" {
			return "", operations.ErrInvalid
		}
		return "/deployment/" + r.PathValue("id") + "/container/" + r.PathValue("container_id"), nil
	}, nil))
	mux.HandleFunc("GET /api/deployments/{id}/logs", handle(false, func(w http.ResponseWriter, r *http.Request, _ *pb.AuthorizeResponse, s *operations.IoNet) {
		q, e := ioNetQuery(r, "container_id", "stream", "limit")
		if e != nil {
			ioNetRespond(w, nil, e)
			return
		}
		limit, e := ioNetPositive(q, "limit", 100, 500)
		if e != nil {
			ioNetRespond(w, nil, e)
			return
		}
		stream := q.Get("stream")
		if stream == "" {
			stream = "stdout"
		}
		v, e := s.Logs(r.Context(), r.PathValue("id"), q.Get("container_id"), stream, limit)
		ioNetRespond(w, v, e)
	}))
	for pattern, kind := range map[string]string{"POST /api/deployments/{$}": "create", "POST /api/deployments/{id}/extend": "extend", "PUT /api/deployments/{id}": "update", "PUT /api/deployments/{id}/name": "rename", "DELETE /api/deployments/{id}": "delete"} {
		mux.HandleFunc(pattern, handle(true, func(w http.ResponseWriter, r *http.Request, a *pb.AuthorizeResponse, s *operations.IoNet) {
			if r.URL.RawQuery != "" {
				ioNetRespond(w, nil, operations.ErrInvalid)
				return
			}
			var raw json.RawMessage
			if kind == "delete" {
				if !operationsEmptyBody(w, r) {
					ioNetRespond(w, nil, operations.ErrInvalid)
					return
				}
				raw = json.RawMessage(`{}`)
			} else {
				if supportDecode(w, r, 131072, &raw, operations.IoNetMutationFields(kind)...) != nil {
					ioNetRespond(w, nil, operations.ErrInvalid)
					return
				}
			}
			intent, e := operations.ParseIoNetMutation(kind, r.PathValue("id"), raw)
			if e != nil {
				ioNetRespond(w, nil, e)
				return
			}
			keys := r.Header.Values("Idempotency-Key")
			if len(keys) != 1 {
				ioNetRespond(w, nil, operations.ErrInvalid)
				return
			}
			v, e := s.Mutation(r.Context(), a.GetUserId(), keys[0], kind, r.PathValue("id"), intent.Resource, intent.Method, intent.Endpoint, intent.Body)
			ioNetRespond(w, v, e)
		}))
	}
}

func ioNetQuery(r *http.Request, allowed ...string) (url.Values, error) {
	q, e := url.ParseQuery(r.URL.RawQuery)
	if e != nil {
		return nil, operations.ErrInvalid
	}
	for name, values := range q {
		found := false
		for _, a := range allowed {
			if name == a {
				found = true
			}
		}
		if !found || len(values) != 1 {
			return nil, operations.ErrInvalid
		}
	}
	return q, nil
}
func ioNetPositive(q url.Values, key string, fallback, max int) (int, error) {
	raw := q.Get(key)
	if raw == "" && fallback > 0 {
		return fallback, nil
	}
	n, e := strconv.Atoi(raw)
	if e != nil || n < 1 || n > max || strconv.Itoa(n) != raw {
		return 0, operations.ErrInvalid
	}
	return n, nil
}
func ioNetListEndpoint(r *http.Request) (string, error) {
	q, e := ioNetQuery(r, "p", "page_size", "status", "keyword")
	if e != nil {
		return "", e
	}
	page, e := ioNetPositive(q, "p", 1, 10000)
	if e != nil {
		return "", e
	}
	size, e := ioNetPositive(q, "page_size", 20, 100)
	if e != nil {
		return "", e
	}
	status := q.Get("status")
	if len(status) > 32 || len(q.Get("keyword")) > 128 {
		return "", operations.ErrInvalid
	}
	values := url.Values{"page": {strconv.Itoa(page - 1)}, "page_size": {strconv.Itoa(size)}, "sort_by": {"created_at"}, "sort_order": {"desc"}}
	if status != "" {
		values.Set("status", status)
	}
	return "/deployments?" + values.Encode(), nil
}
func ioNetListProjection(v any, r *http.Request) (any, error) {
	obj, ok := v.(map[string]any)
	if !ok {
		return nil, operations.ErrIoNetUpstream
	}
	rows, ok := obj["deployments"].([]any)
	if !ok {
		return nil, operations.ErrIoNetUpstream
	}
	// Preserve the fixed legacy search scope: names on this provider page.
	keyword := strings.ToLower(strings.TrimSpace(r.URL.Query().Get("keyword")))
	if keyword != "" {
		filtered := make([]any, 0)
		for _, row := range rows {
			if item, ok := row.(map[string]any); ok {
				if name, ok := item["name"].(string); ok && strings.Contains(strings.ToLower(name), keyword) {
					filtered = append(filtered, row)
				}
			}
		}
		rows = filtered
		obj["total"] = len(rows)
	}
	for _, row := range rows {
		if item, ok := row.(map[string]any); ok {
			ioNetDeploymentProjection(item)
			item["provider"] = "io.net"
			item["deployment_name"] = item["name"]
			item["container_name"] = item["name"]
			if status, ok := item["status"].(string); ok {
				item["status"] = strings.ToLower(status)
			}
		}
	}
	return map[string]any{"items": rows, "total": obj["total"]}, nil
}

func ioNetDeploymentProjection(v any) any {
	if item, ok := v.(map[string]any); ok {
		for _, field := range []string{"created_at", "started_at", "finished_at"} {
			if value, ok := item[field].(string); ok {
				if parsed, err := time.Parse(time.RFC3339Nano, value); err == nil {
					item[field] = parsed.Unix()
				}
			}
		}
	}
	return v
}

func ioNetJSONReply(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}

func ioNetCanonicalURL(r *http.Request) bool {
	p := strings.TrimPrefix(r.URL.Path, "/extensions/v1/operations")
	clean := path.Clean(p)
	relevant := strings.HasPrefix(p, "/api/deployments/") || p == "/api/deployments" || strings.HasPrefix(clean, "/api/deployments/") || clean == "/api/deployments"
	if !relevant {
		return true
	}
	return r.URL.EscapedPath() == r.URL.Path && (p == "/api/deployments/" || p == clean)
}
