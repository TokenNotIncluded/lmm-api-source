// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"time"
)

type nativeActivityReader interface {
	NativeActivity(context.Context, []int64, int64, int64) (community.NativeActivity, error)
}

func (h *handlers) rebuildActivity(w http.ResponseWriter, r *http.Request, a actor) {
	reader, ok := h.s.UserRead.(nativeActivityReader)
	if !ok {
		respond(w, nil, errUnavailable)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 30*time.Second)
	defer cancel()
	cfg, err := loadConfig(ctx, h.s.db, false)
	if err != nil {
		respond(w, nil, err)
		return
	}
	now := h.s.now().Unix()
	from := max(cfg.StartedAt, now-AccountDays*daySeconds)
	to := now - 60
	if to <= from {
		respond(w, map[string]any{"scheduled": false, "rebuilt": false, "status": "waiting", "incomplete": true}, nil)
		return
	}
	rows, err := h.s.db.QueryContext(ctx, `SELECT a.user_id,c.registered_at FROM lmm_acquisition.accounts a JOIN lmm_acquisition.consents s USING(user_id) LEFT JOIN lmm_acquisition.cohort c USING(user_id) WHERE a.consent_version>=2 AND s.allowed AND s.version>=2 ORDER BY a.user_id LIMIT 10001`)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	ids := []int64{}
	registration := map[int64]sql.NullInt64{}
	for rows.Next() {
		var id int64
		var at sql.NullInt64
		if err = rows.Scan(&id, &at); err != nil {
			break
		}
		ids = append(ids, id)
		registration[id] = at
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if len(ids) > 10000 {
		respond(w, nil, errLimit)
		return
	}
	activity := []community.ActivityDay{}
	for start := 0; start < len(ids); start += 100 {
		batch := ids[start:min(start+100, len(ids))]
		users, err := h.s.UserRead.Users(ctx, batch)
		if err != nil {
			respond(w, nil, errUnavailable)
			return
		}
		eligible := []int64{}
		allowed := map[int64]bool{}
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
			case "admin", "superadmin":
				continue
			case "user":
			default:
				respond(w, nil, errUnavailable)
				return
			}
			at := registration[id]
			if u.RegisteredAt != nil {
				at = sql.NullInt64{Int64: *u.RegisteredAt, Valid: true}
			}
			if at.Valid && at.Int64 >= cfg.StartedAt {
				eligible = append(eligible, id)
				allowed[id] = true
			}
		}
		if len(eligible) == 0 {
			continue
		}
		for begin := from; begin < to; begin += 90 * daySeconds {
			end := min(begin+90*daySeconds, to)
			data, err := reader.NativeActivity(ctx, eligible, begin, end)
			if err != nil || !data.NativeOnly {
				respond(w, nil, errUnavailable)
				return
			}
			for _, day := range data.Rows {
				if !allowed[day.UserID] || day.FirstAt < begin || day.LastAt >= end || day.LastAt < day.FirstAt || day.Day != (day.FirstAt/daySeconds)*daySeconds || day.Day != (day.LastAt/daySeconds)*daySeconds {
					respond(w, nil, errUnavailable)
					return
				}
				activity = append(activity, day)
				if len(activity) > 10000 {
					respond(w, nil, errLimit)
					return
				}
			}
		}
	}
	err = h.s.tx(ctx, func(ctx context.Context, tx *sql.Tx) error {
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.activity_state(singleton,started_at,scanned_through,updated_at,status,incomplete) VALUES(true,$1,$1,$2,'native_only',true) ON CONFLICT DO NOTHING`, from, now); err != nil {
			return err
		}
		var started int64
		if err := tx.QueryRowContext(ctx, `SELECT started_at FROM lmm_acquisition.activity_state WHERE singleton FOR UPDATE`).Scan(&started); err != nil {
			return err
		}

		locked := map[int64]bool{}
		// Withdrawal locks the analytics account before deleting derived rows.
		// Lock existing accounts only; never recreate a withdrawn account/head.
		for _, id := range ids {
			var current int64
			err := tx.QueryRowContext(ctx, `SELECT user_id FROM lmm_acquisition.accounts WHERE user_id=$1 FOR UPDATE`, id).Scan(&current)
			if err == sql.ErrNoRows {
				continue
			}
			if err != nil {
				return err
			}
			locked[id] = true
		}

		for _, day := range activity {
			if !locked[day.UserID] {
				continue
			}
			if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.activities(user_id,day,first_at,last_at) SELECT $1,$2,$3,$4 WHERE EXISTS(SELECT 1 FROM lmm_acquisition.accounts a JOIN lmm_acquisition.consents c USING(user_id) WHERE a.user_id=$1 AND a.consent_version>=2 AND c.allowed AND c.version>=2) ON CONFLICT(user_id,day) DO UPDATE SET first_at=least(lmm_acquisition.activities.first_at,EXCLUDED.first_at),last_at=greatest(lmm_acquisition.activities.last_at,EXCLUDED.last_at)`, day.UserID, day.Day, day.FirstAt, day.LastAt); err != nil {
				return err
			}
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.activity_gaps(from_at,through_at) VALUES($1,$2) ON CONFLICT(from_at) DO UPDATE SET through_at=greatest(lmm_acquisition.activity_gaps.through_at,EXCLUDED.through_at)`, from, to); err != nil {
			return err
		}
		_, err := tx.ExecContext(ctx, `UPDATE lmm_acquisition.activity_state SET scanned_through=greatest(scanned_through,$1),updated_at=$2,status='native_only',incomplete=true,gap_from=CASE WHEN gap_from=0 THEN $3 ELSE least(gap_from,$3) END,gap_through=greatest(gap_through,$1) WHERE singleton`, to, now, min(started, from))
		return err
	})
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, map[string]any{"scheduled": false, "rebuilt": true, "status": "native_only", "incomplete": true, "observed_daily_rows": len(activity)}, nil)
}
