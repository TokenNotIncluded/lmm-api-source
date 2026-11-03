// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"encoding/json"
	"net/http"
)

type costScope struct {
	LinkID   string `json:"link_id"`
	From     int64  `json:"from"`
	To       int64  `json:"to"`
	Days     int    `json:"observation_days"`
	Currency string `json:"currency"`
}

func (s costScope) valid(now int64) bool {
	if !hexID(s.LinkID, 32) || s.From <= 0 || s.To <= s.From || s.To > now || s.To-s.From > 366*daySeconds || s.From < now-AccountDays*daySeconds || s.Days < 1 || s.Days > 90 || len(s.Currency) != 3 {
		return false
	}
	for _, c := range s.Currency {
		if c < 'A' || c > 'Z' {
			return false
		}
	}
	return true
}
func (h *handlers) saveCost(w http.ResponseWriter, r *http.Request, a actor) {
	var input struct {
		costScope
		Amount *int64 `json:"amount_micros"`
	}
	now := h.s.now().Unix()
	if decode(w, r, &input, false) != nil || !input.costScope.valid(now) || input.Amount == nil || *input.Amount < 0 || *input.Amount > 1000000000000000 || a.id <= 0 {
		respond(w, nil, ErrInvalid)
		return
	}
	var payload []byte
	err := h.s.db.QueryRowContext(r.Context(), `INSERT INTO lmm_acquisition.costs(link_id,from_at,to_at,observation_days,currency,amount_micros,updated_by,updated_at) SELECT id,$2,$3,$4,$5,$6,$7,$8 FROM lmm_acquisition.links WHERE id=$1 ON CONFLICT(link_id,from_at,to_at,observation_days,currency) DO UPDATE SET amount_micros=excluded.amount_micros,updated_by=excluded.updated_by,updated_at=excluded.updated_at RETURNING json_build_object('id',id,'link_id',link_id,'from',from_at,'to',to_at,'observation_days',observation_days,'currency',currency,'amount_micros',amount_micros,'updated_by',updated_by,'updated_at',updated_at)`, input.LinkID, input.From, input.To, input.Days, input.Currency, *input.Amount, a.id, now).Scan(&payload)
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, json.RawMessage(payload), nil)
}
