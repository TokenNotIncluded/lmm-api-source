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

type linkInput struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Source   string `json:"source"`
	Medium   string `json:"medium"`
	Campaign string `json:"campaign"`
	Content  string `json:"content"`
	Target   string `json:"target"`
	Archived bool   `json:"archived"`
}

func (h *handlers) saveLink(w http.ResponseWriter, r *http.Request, a actor) {
	var input linkInput
	if decode(w, r, &input, false) != nil {
		respond(w, nil, ErrInvalid)
		return
	}
	input.Name = Label(input.Name)
	if input.Name == "" {
		respond(w, nil, ErrInvalid)
		return
	}
	var payload []byte
	var err error
	if input.ID != "" {
		if !hexID(input.ID, 32) {
			respond(w, nil, ErrInvalid)
			return
		}
		err = h.s.db.QueryRowContext(r.Context(), `UPDATE lmm_acquisition.links SET name=$2,archived=$3 WHERE id=$1 AND deleted_at=0 RETURNING row_to_json(links)`, input.ID, input.Name, input.Archived).Scan(&payload)
	} else {
		input.Source = Label(input.Source)
		input.Medium = Label(input.Medium)
		input.Campaign = Label(input.Campaign)
		input.Content = Label(input.Content)
		input.Target = Target(input.Target)
		input.ID = randomID()
		if input.Source == "" || input.Target == "" || input.ID == "" {
			respond(w, nil, ErrInvalid)
			return
		}
		err = h.s.db.QueryRowContext(r.Context(), `INSERT INTO lmm_acquisition.links(id,name,source,medium,campaign,content,target,archived,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING row_to_json(links)`, input.ID, input.Name, input.Source, input.Medium, input.Campaign, input.Content, input.Target, input.Archived, h.s.now().Unix()).Scan(&payload)
	}
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, json.RawMessage(payload), nil)
}
func (h *handlers) deleteLink(w http.ResponseWriter, r *http.Request, a actor) {
	id := r.PathValue("id")
	if !hexID(id, 32) {
		respond(w, nil, ErrInvalid)
		return
	}
	var saved string
	err := h.s.db.QueryRowContext(r.Context(), `UPDATE lmm_acquisition.links SET deleted_at=$2 WHERE id=$1 AND deleted_at=0 RETURNING id`, id, h.s.now().Unix()).Scan(&saved)
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, map[string]any{"id": saved, "deleted": true}, nil)
}

// previewLink describes attribution without creating consent, visits or cookies.
func (h *handlers) previewLink(w http.ResponseWriter, r *http.Request, a actor) {
	id := r.PathValue("id")
	if !hexID(id, 32) {
		respond(w, nil, ErrInvalid)
		return
	}
	var target string
	var payload []byte
	err := h.s.db.QueryRowContext(r.Context(), `SELECT target,json_build_object('link_id',id,'target',target,'source',source,'medium',medium,'campaign',campaign,'content',content,'evidence','promotion_link','referrer_host','','excluded',true,'archived',archived) FROM lmm_acquisition.links WHERE id=$1 AND deleted_at=0`, id).Scan(&target, &payload)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if Target(target) == "" {
		respond(w, nil, ErrInvalid)
		return
	}
	respond(w, json.RawMessage(payload), nil)
}
