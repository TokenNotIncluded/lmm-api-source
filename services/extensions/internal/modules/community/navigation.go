// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"database/sql"
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

const navigationLimit = 64 << 10

var errNavigationConflict = &apiError{http.StatusConflict, "NAVIGATION_CONFLICT", "Navigation changed; reload before saving"}

// ValidateNavigation preserves unknown extension fields and the Web's boolean
// spellings. Navigation controls presentation, never Core authorization.
func ValidateNavigation(raw string) error {
	if len(raw) > navigationLimit {
		return errInvalid
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal([]byte(raw), &fields) != nil || fields == nil {
		return errInvalid
	}
	for _, key := range []string{"directory", "developers", "guide", "scripts", "webmcp", "signal", "challenges", "home", "console", "docs", "about", "pricing", "rankings", "security"} {
		value, exists := fields[key]
		if !exists {
			continue
		}
		if key == "pricing" || key == "rankings" || key == "security" {
			var access map[string]json.RawMessage
			if json.Unmarshal(value, &access) == nil && access != nil {
				for _, field := range []string{"enabled", "requireAuth"} {
					if v, ok := access[field]; ok && !navigationBoolean(v) {
						return errInvalid
					}
				}
				continue
			}
		}
		if !navigationBoolean(value) {
			return errInvalid
		}
	}
	return nil
}
func navigationBoolean(raw json.RawMessage) bool {
	switch string(raw) {
	case "true", "false", "0", "1":
		return true
	}
	var value string
	if json.Unmarshal(raw, &value) != nil {
		return false
	}
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "true", "false", "0", "1":
		return true
	}
	return false
}

type navigationConfig struct {
	Value    string `json:"value"`
	Revision int64  `json:"revision"`
}

func (h *handlers) navigation(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-cache")
	var config navigationConfig
	err := h.s.db.QueryRowContext(r.Context(), `SELECT value,revision FROM lmm_community.navigation WHERE singleton`).Scan(&config.Value, &config.Revision)
	if err != nil || ValidateNavigation(config.Value) != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, config, nil)
}
func (h *handlers) updateNavigation(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "private, no-store")
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if actor.GetPlatformLevel() != 6 || actor.GetCredentialKind() != pb.CredentialKind_CREDENTIAL_KIND_SESSION {
		respond(w, nil, errForbidden)
		return
	}
	var input struct {
		Value         string  `json:"value"`
		ExpectedValue *string `json:"expected_value"`
	}
	if decode(w, r, 512<<10, &input, false) != nil || ValidateNavigation(input.Value) != nil {
		respond(w, nil, errInvalid)
		return
	}
	if r.Method == http.MethodPost {
		respond(w, map[string]any{"saved": false}, nil)
		return
	}
	// One conditional UPDATE makes stale-write rejection atomic in the module
	// database. The original JSON bytes survive editing and offline import.
	var result navigationConfig
	err = h.s.db.QueryRowContext(r.Context(), `UPDATE lmm_community.navigation SET value=$1,revision=revision+CASE WHEN value=$1 THEN 0 ELSE 1 END,updated_by=$2,updated_at=extract(epoch FROM statement_timestamp())::bigint WHERE singleton AND ($3::text IS NULL OR value=$3) RETURNING value,revision`, input.Value, actor.GetUserId(), input.ExpectedValue).Scan(&result.Value, &result.Revision)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			respond(w, nil, errNavigationConflict)
		} else {
			respond(w, nil, errUnavailable)
		}
		return
	}
	respond(w, result, nil)
}
