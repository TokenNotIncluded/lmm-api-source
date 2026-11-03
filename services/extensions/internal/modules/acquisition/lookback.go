// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"net/http"
)

func (h *handlers) lookback(w http.ResponseWriter, r *http.Request, a actor) {
	var input struct {
		Days int `json:"days"`
	}
	if decode(w, r, &input, false) != nil || input.Days < 1 || input.Days > RawDays {
		respond(w, nil, ErrInvalid)
		return
	}
	err := h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var current int
		if err := tx.QueryRowContext(ctx, `SELECT lookback_days FROM lmm_acquisition.config WHERE singleton FOR UPDATE`).Scan(&current); err != nil {
			return err
		}
		now := h.s.now().Unix()
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.attribution_policies(effective_at,lookback_days) SELECT $1,$2 WHERE NOT EXISTS(SELECT 1 FROM lmm_acquisition.attribution_policies)`, now, current); err != nil {
			return err
		}
		if current == input.Days {
			return nil
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.attribution_policies(effective_at,lookback_days) VALUES($1,$2)`, now, input.Days); err != nil {
			return err
		}
		_, err := tx.ExecContext(ctx, `UPDATE lmm_acquisition.config SET lookback_days=$1 WHERE singleton`, input.Days)
		return err
	})
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, map[string]int{"days": input.Days}, nil)
}
