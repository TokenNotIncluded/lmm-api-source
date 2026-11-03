// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/lib/pq"
	"net/http"
	"strconv"
	"time"
)

// Aggregate consenting browsers, not people. Read one consistent analytics
// snapshot while Core supplies current role authority in bounded batches.
func (h *handlers) visitors(w http.ResponseWriter, r *http.Request, a actor) {
	from, e1 := strconv.ParseInt(r.URL.Query().Get("from"), 10, 64)
	to, e2 := strconv.ParseInt(r.URL.Query().Get("to"), 10, 64)
	now := h.s.now().Unix()
	if e1 != nil || e2 != nil || from <= 0 || to <= from || to-from > 366*daySeconds || to > now+60 {
		respond(w, nil, ErrInvalid)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 10*time.Second)
	defer cancel()
	tx, err := h.s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	defer tx.Rollback()
	var started int64
	if err = tx.QueryRowContext(ctx, `SELECT started_at FROM lmm_acquisition.config WHERE singleton`).Scan(&started); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	available := max(started, now-RawDays*daySeconds)
	actual := max(from, available)
	rows, err := tx.QueryContext(ctx, `SELECT DISTINCT b.user_id FROM lmm_acquisition.visits v JOIN lmm_acquisition.visitors b ON b.id=v.visitor_id WHERE v.created_at >= $1 AND v.created_at < $2 AND b.user_id>0 ORDER BY b.user_id`, actual, to)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	ids := []int64{}
	for rows.Next() {
		var id int64
		if err = rows.Scan(&id); err != nil {
			break
		}
		ids = append(ids, id)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	eligible := []int64{}
	for start := 0; start < len(ids); start += 100 {
		if h.s.UserRead == nil {
			respond(w, nil, errUnavailable)
			return
		}
		batch := ids[start:min(start+100, len(ids))]
		users, err := h.s.UserRead.Users(ctx, batch)
		if err != nil {
			respond(w, nil, errUnavailable)
			return
		}
		for _, id := range batch {
			u, ok := users[id]
			if !ok {
				continue
			}
			if u.UserID != id {
				respond(w, nil, errUnavailable)
				return
			}
			switch u.PlatformRole {
			case "user":
				eligible = append(eligible, id)
			case "admin", "superadmin":
			default:
				respond(w, nil, errUnavailable)
				return
			}
		}
	}
	var payload json.RawMessage
	err = tx.QueryRowContext(ctx, `WITH observed AS (SELECT v.visitor_id,v.source FROM lmm_acquisition.visits v JOIN lmm_acquisition.visitors b ON b.id=v.visitor_id WHERE v.created_at >= $1 AND v.created_at < $2 AND (b.user_id=0 OR b.user_id=ANY($3::bigint[]))), channels AS (SELECT source,count(DISTINCT visitor_id) AS visitors FROM observed GROUP BY source ORDER BY visitors DESC,source ASC) SELECT json_build_object('from',$4::bigint,'to',$2::bigint,'available_from',$5::bigint,'coverage_complete',$4::bigint >= $5::bigint,'observed_visitors',(SELECT count(DISTINCT visitor_id) FROM observed),'channels',coalesce((SELECT json_agg(channels) FROM channels),'[]'::json))`, actual, to, pq.Array(eligible), from, available).Scan(&payload)
	if err == nil {
		err = tx.Commit()
	}
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, payload, nil)
}
