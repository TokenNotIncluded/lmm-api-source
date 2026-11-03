// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"strconv"
	"time"
)

type registrationReader interface {
	UserPage(context.Context, int64, int) (community.RegistrationPage, error)
}

func (h *handlers) users(w http.ResponseWriter, r *http.Request, a actor) {
	from, e1 := strconv.ParseInt(r.URL.Query().Get("from"), 10, 64)
	to, e2 := strconv.ParseInt(r.URL.Query().Get("to"), 10, 64)
	page := 1
	if raw := r.URL.Query().Get("page"); raw != "" {
		var err error
		page, err = strconv.Atoi(raw)
		if err != nil {
			respond(w, nil, ErrInvalid)
			return
		}
	}
	source := r.URL.Query().Get("source")
	if e1 != nil || e2 != nil || from <= 0 || to <= from || to-from > 366*daySeconds || page < 1 || page > 100000 || len(source) > 253 {
		respond(w, nil, ErrInvalid)
		return
	}
	reader, ok := h.s.UserRead.(registrationReader)
	if !ok {
		respond(w, nil, errUnavailable)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	users := []community.RegistrationUser{}
	cursor := int64(0)
	scanned := 0
	for {
		batch, err := reader.UserPage(ctx, cursor, 100)
		if err != nil || len(batch.Users) > 100 {
			respond(w, nil, errUnavailable)
			return
		}
		for _, user := range batch.Users {
			if user.UserID <= cursor {
				respond(w, nil, errUnavailable)
				return
			}
			cursor = user.UserID
			scanned++
			if scanned > 10000 {
				respond(w, nil, errLimit)
				return
			}
			switch user.PlatformRole {
			case "user":
				users = append(users, user)
			case "admin", "superadmin":
			default:
				respond(w, nil, errUnavailable)
				return
			}
		}
		if !batch.HasMore {
			break
		}
		if len(batch.Users) == 0 || batch.NextID != cursor {
			respond(w, nil, errUnavailable)
			return
		}
	}
	raw, err := json.Marshal(users)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	var result json.RawMessage
	err = h.s.db.QueryRowContext(ctx, `WITH identity AS (SELECT * FROM jsonb_to_recordset($1::jsonb) AS u(user_id bigint,registered_at bigint)), cohort AS (SELECT u.user_id,coalesce(u.registered_at,c.registered_at) AS registered_at FROM identity u LEFT JOIN lmm_acquisition.cohort c USING(user_id)), attributed AS (SELECT u.user_id,u.registered_at,coalesce(a.registration_source,CASE WHEN u.registered_at < f.started_at THEN 'historical_unrecorded' ELSE 'unknown' END) AS source,coalesce(a.registration_evidence,'unavailable') AS evidence,(SELECT coalesce(min(first_at),0) FROM lmm_acquisition.activities WHERE user_id=u.user_id) AS first_success_at FROM cohort u CROSS JOIN lmm_acquisition.config f LEFT JOIN lmm_acquisition.accounts a ON a.user_id=u.user_id AND a.created_at >= $5 WHERE u.registered_at >= $2 AND u.registered_at < $3), filtered AS (SELECT * FROM attributed WHERE $4='' OR source=$4), page AS (SELECT * FROM filtered ORDER BY registered_at DESC,user_id DESC LIMIT 50 OFFSET $6) SELECT json_build_object('items',coalesce((SELECT json_agg(page) FROM page),'[]'::json),'total',(SELECT count(*) FROM filtered),'page',$7::integer,'unknown_registration_count',(SELECT count(*) FROM cohort WHERE registered_at IS NULL),'coverage_complete',NOT EXISTS(SELECT 1 FROM cohort WHERE registered_at IS NULL))`, string(raw), from, to, source, h.s.now().Unix()-AccountDays*daySeconds, (page-1)*50, page).Scan(&result)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, result, nil)
}
