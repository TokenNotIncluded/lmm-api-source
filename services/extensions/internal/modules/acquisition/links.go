// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
	"unicode/utf8"
)

func (h *handlers) links(w http.ResponseWriter, r *http.Request, a actor) {
	page, _ := strconv.Atoi(r.URL.Query().Get("page"))
	if page < 1 {
		page = 1
	}
	size := 100
	if raw := r.URL.Query().Get("page_size"); raw != "" {
		size, _ = strconv.Atoi(raw)
	}
	search := strings.TrimSpace(r.URL.Query().Get("q"))
	status := r.URL.Query().Get("status")
	if status == "" {
		status = "all"
	}
	if page > 100000 || size < 1 || size > 100 || utf8.RuneCountInString(search) > 80 {
		respond(w, nil, ErrInvalid)
		return
	}
	where := "deleted_at=0"
	switch status {
	case "all":
	case "active":
		where += " AND NOT archived"
	case "archived":
		where += " AND archived"
	case "deleted":
		where = "deleted_at>0"
	default:
		respond(w, nil, ErrInvalid)
		return
	}
	search = strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(strings.ToLower(search))
	where += ` AND (lower(name) LIKE $1 ESCAPE '\' OR lower(source) LIKE $1 ESCAPE '\' OR lower(campaign) LIKE $1 ESCAPE '\')`
	var payload []byte
	err := h.s.db.QueryRowContext(r.Context(), `WITH filtered AS (SELECT * FROM lmm_acquisition.links WHERE `+where+`), page AS (SELECT * FROM filtered ORDER BY created_at DESC,id DESC LIMIT $2 OFFSET $3) SELECT json_build_object('items',coalesce((SELECT json_agg(page) FROM page),'[]'::json),'total',(SELECT count(*) FROM filtered),'page',$4::integer,'page_size',$2::integer)`, "%"+search+"%", size, (page-1)*size, page).Scan(&payload)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, json.RawMessage(payload), nil)
}
