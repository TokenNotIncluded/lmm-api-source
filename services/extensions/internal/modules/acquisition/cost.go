// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"strconv"
	"time"
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

type firstCashReader interface {
	FirstCashPayments(context.Context, []int64) (community.FirstCashPayments, error)
}

type costReport struct {
	Scope costScope `json:"scope"`
	Spend *struct {
		Amount int64 `json:"amount_micros"`
	} `json:"spend"`
	Registrations       int    `json:"registrations"`
	FirstPayingAccounts int    `json:"first_paying_accounts"`
	Unclassified        int64  `json:"payment_rows_unclassified"`
	Observing           bool   `json:"observing"`
	CompleteAfter       int64  `json:"complete_after"`
	ObservedUntil       int64  `json:"observed_until"`
	PerRegistration     *int64 `json:"cost_per_registration_micros"`
	PerPayer            *int64 `json:"cost_per_first_payer_micros"`
}

func (h *handlers) cost(w http.ResponseWriter, r *http.Request, a actor) {
	q := r.URL.Query()
	from, e1 := strconv.ParseInt(q.Get("from"), 10, 64)
	to, e2 := strconv.ParseInt(q.Get("to"), 10, 64)
	days, e3 := strconv.Atoi(q.Get("observation_days"))
	scope := costScope{LinkID: q.Get("link_id"), From: from, To: to, Days: days, Currency: q.Get("currency")}
	now := h.s.now().Unix()
	if e1 != nil || e2 != nil || e3 != nil || !scope.valid(now) {
		respond(w, nil, ErrInvalid)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	identity, err := h.registrationSnapshot(ctx)
	if err != nil {
		respond(w, nil, err)
		return
	}
	tx, err := h.s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	defer tx.Rollback()
	var exists bool
	if err = tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM lmm_acquisition.links WHERE id=$1)`, scope.LinkID).Scan(&exists); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if !exists {
		respond(w, nil, errNotFound)
		return
	}
	var spend sql.NullInt64
	err = tx.QueryRowContext(ctx, `SELECT (SELECT amount_micros FROM lmm_acquisition.costs WHERE link_id=$1 AND from_at=$2 AND to_at=$3 AND observation_days=$4 AND currency=$5)`, scope.LinkID, from, to, days, scope.Currency).Scan(&spend)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	rows, err := tx.QueryContext(ctx, `WITH identity AS (SELECT * FROM jsonb_to_recordset($1::jsonb) AS u(user_id bigint,registered_at bigint)), attributed AS (SELECT u.user_id,coalesce(u.registered_at,c.registered_at) AS registered_at FROM identity u LEFT JOIN lmm_acquisition.cohort c USING(user_id) JOIN lmm_acquisition.accounts a USING(user_id) WHERE a.registration_link_id=$2 AND a.created_at>=$5) SELECT user_id,registered_at FROM attributed WHERE registered_at IS NULL OR (registered_at>=$3 AND registered_at<$4) ORDER BY user_id`, string(identity), scope.LinkID, from, to, now-AccountDays*daySeconds)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	ids := []int64{}
	registered := map[int64]int64{}
	for rows.Next() {
		var id int64
		var at sql.NullInt64
		if err = rows.Scan(&id, &at); err != nil {
			break
		}
		if !at.Valid {
			err = errUnavailable
			break
		}
		ids = append(ids, id)
		registered[id] = at.Int64
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	result := costReport{Scope: scope, Registrations: len(ids), CompleteAfter: to + int64(days)*daySeconds, ObservedUntil: now}
	result.Observing = now < result.CompleteAfter
	observations := make([]funnelObservation, 0, len(ids))
	for start := 0; start < len(ids); start += 100 {
		reader, ok := h.s.UserRead.(firstCashReader)
		if !ok {
			respond(w, nil, errUnavailable)
			return
		}
		batch := ids[start:min(start+100, len(ids))]
		facts, e := reader.FirstCashPayments(ctx, batch)
		if e != nil || len(facts.Users) != len(batch) {
			respond(w, nil, errUnavailable)
			return
		}
		requested := map[int64]bool{}
		for _, id := range batch {
			requested[id] = true
		}
		for _, fact := range facts.Users {
			if !requested[fact.UserID] || fact.UnclassifiedOrders < 0 || (!fact.HistoryComplete && fact.UnclassifiedOrders == 0) || (fact.HistoryComplete && fact.UnclassifiedOrders != 0) {
				respond(w, nil, errUnavailable)
				return
			}
			delete(requested, fact.UserID)
			result.Unclassified += fact.UnclassifiedOrders
			observation := funnelObservation{RegisteredAt: registered[fact.UserID], Complete: fact.HistoryComplete}
			if fact.HistoryComplete {
				observation.At = fact.FirstPaidAt
				observation.FirstIsExact = true
			}
			observations = append(observations, observation)
		}
	}
	stage, err := summarizeFunnelStage("first_payment", now, days, observations)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	result.FirstPayingAccounts = stage.Observed
	if err = tx.Commit(); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if spend.Valid {
		result.Spend = &struct {
			Amount int64 `json:"amount_micros"`
		}{spend.Int64}
		if result.Registrations > 0 {
			value := (spend.Int64 + int64(result.Registrations)/2) / int64(result.Registrations)
			result.PerRegistration = &value
		}
		if !result.Observing && result.Unclassified == 0 && result.FirstPayingAccounts > 0 {
			value := (spend.Int64 + int64(result.FirstPayingAccounts)/2) / int64(result.FirstPayingAccounts)
			result.PerPayer = &value
		}
	}
	respond(w, result, nil)
}
