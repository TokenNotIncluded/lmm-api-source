package checkins

import (
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"io"
	"net/http"
	"net/url"
	"strings"
)

func (s *Service) Name() string { return "checkins" }
func (s *Service) Handler() http.Handler {
	mux := http.NewServeMux()
	s.Register(mux)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		copy := r.Clone(r.Context())
		if credential := r.Header.Values("X-LMM-User-Credential"); len(credential) == 1 && len(r.Header.Values("Authorization")) == 0 {
			copy.Header = r.Header.Clone()
			copy.Header.Set("Authorization", "Bearer "+credential[0])
			copy.Header.Del("X-LMM-User-Credential")
		}
		mux.ServeHTTP(w, copy)
	})
}
func (s *Service) Register(mux modules.RouteRegistrar) {
	mux.HandleFunc("GET /api/checkin/capabilities", s.capabilitiesHTTP)
	mux.HandleFunc("GET /api/user/checkin", s.statusHTTP)
	mux.HandleFunc("POST /api/user/checkin", s.claimHTTP)
}
func credential(r *http.Request) (string, error) {
	v := r.Header.Values("Authorization")
	if len(v) != 1 {
		return "", ErrUnauthorized
	}
	scheme, token, ok := strings.Cut(v[0], " ")
	if !ok || !strings.EqualFold(scheme, "Bearer") || token == "" || strings.ContainsAny(token, " \t\r\n") {
		return "", ErrUnauthorized
	}
	return token, nil
}
func write(w http.ResponseWriter, data any, e error) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "private, no-store")
	w.Header().Set("Vary", "Authorization")
	status := 200
	code := ""
	switch e {
	case nil:
	case ErrInvalid:
		status = 400
		code = "CHECKIN_INVALID"
	case ErrUnauthorized:
		status = 401
		code = "CHECKIN_UNAUTHORIZED"
	case ErrForbidden:
		status = 403
		code = "CHECKIN_FORBIDDEN"
	case ErrConflict:
		status = 409
		code = "CHECKIN_CONFLICT"
	default:
		status = 503
		code = "CHECKIN_UNAVAILABLE"
	}
	w.WriteHeader(status)
	if code == "CHECKIN_UNAVAILABLE" {
		e = ErrUnavailable
	}
	if e != nil {
		json.NewEncoder(w).Encode(map[string]any{"success": false, "message": e.Error(), "error": map[string]string{"code": code}})
	} else {
		json.NewEncoder(w).Encode(map[string]any{"success": true, "data": data})
	}
}
func (s *Service) statusHTTP(w http.ResponseWriter, r *http.Request) {
	user, e := credential(r)
	if e != nil {
		write(w, nil, e)
		return
	}
	q, e := url.ParseQuery(r.URL.RawQuery)
	if e != nil {
		write(w, nil, ErrInvalid)
		return
	}
	for key, v := range q {
		if key != "month" || len(v) != 1 {
			write(w, nil, ErrInvalid)
			return
		}
	}
	result, e := s.Status(r.Context(), user, q.Get("month"))
	write(w, result, e)
}
func (s *Service) claimHTTP(w http.ResponseWriter, r *http.Request) {
	user, e := credential(r)
	if e != nil {
		write(w, nil, e)
		return
	}
	q, e := url.ParseQuery(r.URL.RawQuery)
	if e != nil {
		write(w, nil, ErrInvalid)
		return
	}
	for key, v := range q {
		if key != "turnstile" || len(v) != 1 {
			write(w, nil, ErrInvalid)
			return
		}
		if v[0] != "" {
			write(w, nil, ErrUnavailable)
			return
		}
	}
	d := json.NewDecoder(http.MaxBytesReader(w, r.Body, 16384))
	start, e := d.Token()
	if e != nil || start != json.Delim('{') {
		write(w, nil, ErrInvalid)
		return
	}
	var input ClaimRequest
	seen := false
	for d.More() {
		key, e := d.Token()
		if e != nil || key != "operation_key" || seen || d.Decode(&input.OperationKey) != nil {
			write(w, nil, ErrInvalid)
			return
		}
		seen = true
	}
	end, e := d.Token()
	var extra any
	if e != nil || end != json.Delim('}') || d.Decode(&extra) != io.EOF || !seen {
		write(w, nil, ErrInvalid)
		return
	}
	result, e := s.Claim(r.Context(), user, input)
	write(w, result, e)
}

// Public capability discovery reads only this module's published business policy.
// It exposes no amount, user status, Core credential or database metadata.
func (s *Service) capabilitiesHTTP(w http.ResponseWriter, r *http.Request) {
	if r.URL.RawQuery != "" {
		write(w, nil, ErrInvalid)
		return
	}
	tx, e := s.begin(r.Context())
	if e != nil {
		write(w, nil, e)
		return
	}
	defer tx.Rollback()
	p, _, e := loadPolicy(r.Context(), tx)
	if e != nil {
		write(w, nil, e)
		return
	}
	if tx.Commit() != nil {
		write(w, nil, ErrUnavailable)
		return
	}
	write(w, struct {
		Enabled bool `json:"enabled"`
	}{Enabled: p != nil && p.Enabled}, nil)
}
