// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"sort"
	"strconv"
	"time"
)

type cashTotalsReader interface {
	firstCashReader
	ReadyCashPaymentTotals(context.Context) error
	CashPaymentTotals(context.Context, []int64, int64, int64) (community.CashPaymentTotals, error)
}
type reportChannel struct {
	Source        string `json:"source"`
	Evidence      string `json:"evidence"`
	Registrations int    `json:"registrations"`
	Identified    int    `json:"identified_registrations"`
}
type reportPayment struct {
	Source   string `json:"source"`
	Currency string `json:"currency"`
	Paid     int64  `json:"paid_micros"`
	Refund   int64  `json:"refund_micros"`
	Net      int64  `json:"net_micros"`
	Payers   int    `json:"paying_accounts"`
}
type reportActivityState struct {
	Started    int64  `json:"started_at"`
	Through    int64  `json:"scanned_through"`
	Updated    int64  `json:"updated_at"`
	Status     string `json:"status"`
	Incomplete bool   `json:"incomplete"`
}
type reportActivity struct {
	Source     string   `json:"source"`
	Eligible   int      `json:"eligible_accounts"`
	Successful int      `json:"successful_accounts"`
	Incomplete int      `json:"incomplete_accounts"`
	Mature     int      `json:"mature_accounts"`
	Retained   int      `json:"retained_accounts"`
	Observing  int      `json:"observing_accounts"`
	Rate       *float64 `json:"retention_rate"`
}
type acquisitionReport struct {
	From         int64                `json:"from"`
	To           int64                `json:"to"`
	Until        int64                `json:"observed_until"`
	Started      int64                `json:"started_at"`
	Lookback     int                  `json:"lookback_days"`
	Applied      []int                `json:"applied_lookback_days"`
	Channels     []reportChannel      `json:"channels"`
	Payments     []reportPayment      `json:"payments"`
	Unclassified int64                `json:"unclassified_payment_rows"`
	State        *reportActivityState `json:"activity_state"`
	Activity     []reportActivity     `json:"activity"`
}

func (h *handlers) report(w http.ResponseWriter, r *http.Request, a actor) {
	from, e1 := strconv.ParseInt(r.URL.Query().Get("from"), 10, 64)
	to, e2 := strconv.ParseInt(r.URL.Query().Get("to"), 10, 64)
	now := h.s.now().Unix()
	if e1 != nil || e2 != nil || from <= 0 || to <= from || from >= now || to-from > 366*daySeconds || to > now+60 {
		respond(w, nil, ErrInvalid)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	raw, err := h.registrationSnapshot(ctx)
	if err != nil {
		respond(w, nil, err)
		return
	}
	reader, ok := h.s.UserRead.(cashTotalsReader)
	if !ok || reader.ReadyCashPaymentTotals(ctx) != nil {
		respond(w, nil, errUnavailable)
		return
	}
	var users []community.RegistrationUser
	if json.Unmarshal(raw, &users) != nil {
		respond(w, nil, errUnavailable)
		return
	}
	tx, err := h.s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	defer tx.Rollback()
	cfg, err := loadConfig(ctx, tx, false)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	out := acquisitionReport{From: from, To: to, Until: now, Started: cfg.StartedAt, Lookback: cfg.LookbackDays, Applied: []int{}, Channels: []reportChannel{}, Payments: []reportPayment{}, Activity: []reportActivity{}}
	var state reportActivityState
	err = tx.QueryRowContext(ctx, `SELECT started_at,scanned_through,updated_at,status,incomplete FROM lmm_acquisition.activity_state WHERE singleton`).Scan(&state.Started, &state.Through, &state.Updated, &state.Status, &state.Incomplete)
	if err == nil {
		out.State = &state
	} else if err != sql.ErrNoRows {
		respond(w, nil, errUnavailable)
		return
	}
	var data []byte
	err = tx.QueryRowContext(ctx, registrationQuery+`, selected AS (
 SELECT f.*,coalesce(a.lookback_days,0) AS lookback_days,
 coalesce(a.consent_version>=2 AND c.allowed AND c.version>=2,false) AND f.registered_at>=$6 AND f.registered_at>=$5 AS eligible,
 EXISTS(SELECT 1 FROM lmm_acquisition.activities d WHERE d.user_id=f.user_id AND d.day=(f.first_success_at/86400)*86400+7*86400) AS retained,
 EXISTS(SELECT 1 FROM lmm_acquisition.activity_gaps g WHERE g.from_at<CASE WHEN f.first_success_at>0 THEN least($7::bigint,(f.first_success_at/86400)*86400+8*86400) ELSE $7::bigint END AND g.through_at>f.registered_at) AS gap
 FROM filtered f LEFT JOIN lmm_acquisition.accounts a ON a.user_id=f.user_id AND a.created_at>=$5 LEFT JOIN lmm_acquisition.consents c ON c.user_id=f.user_id)
 SELECT json_build_object('unknown',EXISTS(SELECT 1 FROM cohort WHERE registered_at IS NULL),'items',coalesce((SELECT json_agg(selected ORDER BY user_id) FROM selected),'[]'::json))`, string(raw), from, to, "", now-AccountDays*daySeconds, cfg.StartedAt, now).Scan(&data)
	var doc struct {
		Unknown bool
		Items   []struct {
			funnelAccount
			Evidence string
			Lookback int `json:"lookback_days"`
			Eligible bool
			Retained bool
			Gap      bool
		}
	}
	if err != nil || json.Unmarshal(data, &doc) != nil || doc.Unknown {
		respond(w, nil, errUnavailable)
		return
	}
	channels := map[[2]string]*reportChannel{}
	activity := map[string]*reportActivity{}
	lookbacks := map[int]bool{}
	for _, u := range doc.Items {
		key := [2]string{u.Source, u.Evidence}
		if channels[key] == nil {
			channels[key] = &reportChannel{Source: u.Source, Evidence: u.Evidence}
		}
		v := channels[key]
		v.Registrations++
		if u.Source != "unknown" && u.Source != "historical_unrecorded" && (u.Evidence == "promotion_link" || u.Evidence == "campaign_parameters" || u.Evidence == "browser_referrer") {
			v.Identified++
		}
		if u.Lookback >= 1 && u.Lookback <= 90 {
			lookbacks[u.Lookback] = true
		}
		if !u.Eligible {
			continue
		}
		if activity[u.Source] == nil {
			activity[u.Source] = &reportActivity{Source: u.Source}
		}
		v2 := activity[u.Source]
		v2.Eligible++
		if u.Success > now || (u.Success > 0 && u.Success < u.Registered) {
			respond(w, nil, errUnavailable)
			return
		}
		if u.Success > 0 {
			v2.Successful++
		}
		if out.State == nil || state.Incomplete || state.Started > u.Registered || u.Gap {
			v2.Incomplete++
			continue
		}
		if u.Success == 0 {
			continue
		}
		end := (u.Success/daySeconds)*daySeconds + 8*daySeconds
		if end > now || state.Through < end {
			v2.Observing++
			continue
		}
		v2.Mature++
		if u.Retained {
			v2.Retained++
		}
	}
	for _, c := range channels {
		out.Channels = append(out.Channels, *c)
	}
	sort.Slice(out.Channels, func(i, j int) bool {
		a, b := out.Channels[i], out.Channels[j]
		if a.Source != b.Source {
			return a.Source < b.Source
		}
		return a.Evidence < b.Evidence
	})
	for days := range lookbacks {
		out.Applied = append(out.Applied, days)
	}
	sort.Ints(out.Applied)
	for _, v := range activity {
		if v.Mature > 0 {
			rate := float64(v.Retained) / float64(v.Mature)
			v.Rate = &rate
		}
		out.Activity = append(out.Activity, *v)
	}
	sort.Slice(out.Activity, func(i, j int) bool { return out.Activity[i].Source < out.Activity[j].Source })
	type snapshot struct {
		At     int64
		Source string
	}
	snapshots := map[int64]snapshot{}
	rows, err := tx.QueryContext(ctx, `SELECT user_id,first_paid_at,source FROM lmm_acquisition.first_payments WHERE history_conflict_at=0 AND snapshotted_at>0 AND first_paid_at>=$1 AND lookback_days BETWEEN 1 AND 90`, now-AccountDays*daySeconds)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	for rows.Next() {
		var id, at int64
		var source string
		if err = rows.Scan(&id, &at, &source); err != nil {
			break
		}
		snapshots[id] = snapshot{at, source}
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	sums := map[[2]string]*reportPayment{}
	const safe = int64(9007199254740991)
	for start := 0; start < len(users); start += 100 {
		batch := users[start:min(start+100, len(users))]
		ids := []int64{}
		wanted := map[int64]bool{}
		for _, u := range batch {
			ids = append(ids, u.UserID)
			wanted[u.UserID] = true
		}
		totals, e := reader.CashPaymentTotals(ctx, ids, from, min(to, now))
		if e != nil || len(totals.Users) != len(ids) {
			respond(w, nil, errUnavailable)
			return
		}
		first, e := reader.FirstCashPayments(ctx, ids)
		if e != nil || len(first.Users) != len(ids) {
			respond(w, nil, errUnavailable)
			return
		}
		paymentSources := map[int64]string{}
		for _, f := range first.Users {
			if !wanted[f.UserID] {
				respond(w, nil, errUnavailable)
				return
			}
			delete(wanted, f.UserID)
			saved := snapshots[f.UserID]
			source := "unknown"
			if f.HistoryComplete && f.FirstPaidAt != nil && saved.At == *f.FirstPaidAt && saved.Source != "" {
				source = saved.Source
			}
			paymentSources[f.UserID] = source
		}
		seen := map[int64]bool{}
		for _, u := range totals.Users {
			source, ok := paymentSources[u.UserID]
			if !ok || seen[u.UserID] || u.UnclassifiedOrders < 0 || u.UnclassifiedOrders > safe-out.Unclassified {
				respond(w, nil, errUnavailable)
				return
			}
			seen[u.UserID] = true
			out.Unclassified += u.UnclassifiedOrders
			currencies := map[string]bool{}
			for _, c := range u.Currencies {
				if currencies[c.Currency] || c.Orders <= 0 || c.PaidMicros <= 0 || c.RefundMicros < 0 || c.RefundMicros > c.PaidMicros || c.NetMicros != c.PaidMicros-c.RefundMicros {
					respond(w, nil, errUnavailable)
					return
				}
				currencies[c.Currency] = true
				key := [2]string{source, c.Currency}
				if sums[key] == nil {
					sums[key] = &reportPayment{Source: source, Currency: c.Currency}
				}
				v := sums[key]
				if c.PaidMicros > safe-v.Paid || c.RefundMicros > safe-v.Refund {
					respond(w, nil, errUnavailable)
					return
				}
				v.Paid += c.PaidMicros
				v.Refund += c.RefundMicros
				v.Net = v.Paid - v.Refund
				v.Payers++
			}
		}
	}
	for _, v := range sums {
		out.Payments = append(out.Payments, *v)
	}
	sort.Slice(out.Payments, func(i, j int) bool {
		a, b := out.Payments[i], out.Payments[j]
		if a.Source != b.Source {
			return a.Source < b.Source
		}
		return a.Currency < b.Currency
	})
	if err = tx.Commit(); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, out, nil)
}
