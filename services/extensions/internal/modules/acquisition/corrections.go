// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"encoding/json"
	"net/http"
	"strconv"
)

func (h *handlers) correctionTarget(r *http.Request) (int64, error) {
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil || id <= 0 {
		return 0, ErrInvalid
	}
	if h.s.UserRead == nil {
		return 0, errUnavailable
	}
	users, err := h.s.UserRead.Users(r.Context(), []int64{id})
	if err != nil {
		return 0, errUnavailable
	}
	user, ok := users[id]
	if !ok {
		return 0, errNotFound
	}
	if user.UserID != id {
		return 0, errUnavailable
	}
	if !user.Active {
		return 0, errNotFound
	}
	switch user.PlatformRole {
	case "user":
		return id, nil
	case "admin", "superadmin":
		return 0, errForbidden
	default:
		return 0, errUnavailable
	}
}
func (h *handlers) corrections(w http.ResponseWriter, r *http.Request, a actor) {
	id, err := h.correctionTarget(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	cutoff := h.s.now().Unix() - AccountDays*daySeconds
	var payload []byte
	err = h.s.db.QueryRowContext(r.Context(), `WITH recent AS (SELECT * FROM lmm_acquisition.corrections WHERE user_id=$1 AND created_at >= $2 ORDER BY id DESC LIMIT 101), page AS (SELECT * FROM recent ORDER BY id DESC LIMIT 100) SELECT json_build_object('head',(SELECT row_to_json(h) FROM lmm_acquisition.correction_heads h WHERE user_id=$1 AND revision>0 AND updated_at >= $2),'items',coalesce((SELECT json_agg(page) FROM page),'[]'::json),'has_more',(SELECT count(*)>100 FROM recent))`, id, cutoff).Scan(&payload)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, json.RawMessage(payload), nil)
}
