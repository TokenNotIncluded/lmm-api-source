// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"sort"
	"strconv"
	"time"
)

type funnelAccount struct {
	ID              int64  `json:"user_id"`
	Registered      int64  `json:"registered_at"`
	Source          string `json:"source"`
	Success         int64  `json:"first_success_at"`
	SuccessComplete bool   `json:"success_complete"`
}
type funnelChannel struct {
	Source        string        `json:"source"`
	Registrations int           `json:"registrations"`
	Mature        int           `json:"mature_accounts"`
	Observing     int           `json:"observing_accounts"`
	Stages        []funnelStage `json:"stages"`
}
type paymentSource struct {
	Source   string `json:"source"`
	Evidence string `json:"evidence"`
	Rule     string `json:"rule"`
	Days     int    `json:"lookback_days"`
	Inferred bool   `json:"inferred"`
	Accounts int    `json:"accounts"`
}
type funnelReport struct {
	Until          int64           `json:"observed_until"`
	Days           int             `json:"observation_days"`
	Unavailable    []string        `json:"unavailable"`
	PaymentUpdated int64           `json:"payment_snapshot_updated_at"`
	PaymentStatus  string          `json:"payment_snapshot_status"`
	Channels       []funnelChannel `json:"channels"`
	Sources        []paymentSource `json:"first_payment_sources"`
}
type repeatCashReader interface {
	firstCashReader
	ReadyRepeatCashPayments(context.Context) error
}

func (h *handlers) funnel(w http.ResponseWriter, r *http.Request, a actor) {
	q := r.URL.Query()
	from, e1 := strconv.ParseInt(q.Get("from"), 10, 64)
	to, e2 := strconv.ParseInt(q.Get("to"), 10, 64)
	days, e3 := strconv.Atoi(q.Get("observation_days"))
	now := h.s.now().Unix()
	source, campaign, content := q.Get("source"), q.Get("campaign"), q.Get("content")
	if e1 != nil || e2 != nil || e3 != nil || from <= 0 || to <= from || to-from > 366*daySeconds || to > now+60 || days < 1 || days > 90 || len(source) > 253 || len(campaign) > 80 || len(content) > 80 {
		respond(w, nil, ErrInvalid)
		return
	}
	switch q.Get("connection_method") {
	case "":
	case "oauth", "api_key", "both", "unknown":
		respond(w, nil, errUnavailable)
		return
	default:
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
	reader, ok := h.s.UserRead.(repeatCashReader)
	if !ok || reader.ReadyRepeatCashPayments(ctx) != nil {
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
	var doc struct {
		Unknown bool
		Items   []funnelAccount
	}
	var payload []byte
	err = tx.QueryRowContext(ctx, registrationQuery+`, selected AS (
 SELECT f.*,coalesce((SELECT NOT s.incomplete AND s.started_at<=f.registered_at AND s.scanned_through>=least($8::bigint,f.registered_at+$9::bigint)
 AND f.registered_at >= $5
 AND NOT EXISTS(SELECT 1 FROM lmm_acquisition.activity_gaps g WHERE g.from_at<least($8::bigint,f.registered_at+$9::bigint) AND g.through_at>f.registered_at)
 FROM lmm_acquisition.activity_state s WHERE singleton),false)
 AND coalesce(a.consent_version>=2 AND c.allowed AND c.version>=2,false) AS success_complete
 FROM filtered f LEFT JOIN lmm_acquisition.accounts a ON a.user_id=f.user_id AND a.created_at >= $5
 LEFT JOIN lmm_acquisition.consents c ON c.user_id=f.user_id
 WHERE ($6='' OR a.registration_campaign=$6) AND ($7='' OR a.registration_content=$7))
 SELECT json_build_object('unknown',EXISTS(SELECT 1 FROM cohort WHERE registered_at IS NULL),'items',coalesce((SELECT json_agg(selected ORDER BY user_id) FROM selected),'[]'::json))`, string(raw), from, to, source, now-AccountDays*daySeconds, campaign, content, now, int64(days)*daySeconds).Scan(&payload)
	if err != nil || json.Unmarshal(payload, &doc) != nil || doc.Unknown {
		respond(w, nil, errUnavailable)
		return
	}
	stageIDs := []string{"application_submitted", "access_approved", "oauth_authorized", "api_key_created", "credential_ready", "client_configured", "first_successful_request", "first_payment", "repeat_payment"}
	observations := map[string]map[int64]funnelObservation{}
	for _, stage := range stageIDs {
		observations[stage] = map[int64]funnelObservation{}
	}
	byID := map[int64]funnelAccount{}
	for _, u := range doc.Items {
		byID[u.ID] = u
		for _, stage := range stageIDs {
			observations[stage][u.ID] = funnelObservation{RegisteredAt: u.Registered}
		}
		v := funnelObservation{RegisteredAt: u.Registered, Complete: u.SuccessComplete, FirstIsExact: u.SuccessComplete}
		if u.Success > 0 {
			at := u.Success
			v.At = &at
		}
		observations["first_successful_request"][u.ID] = v
	}
	firstDates := map[int64]int64{}
	for start := 0; start < len(doc.Items); start += 100 {
		batch := doc.Items[start:min(start+100, len(doc.Items))]
		ids := []int64{}
		requested := map[int64]bool{}
		for _, u := range batch {
			ids = append(ids, u.ID)
			requested[u.ID] = true
		}
		facts, e := reader.FirstCashPayments(ctx, ids)
		if e != nil || len(facts.Users) != len(ids) {
			respond(w, nil, errUnavailable)
			return
		}
		for _, fact := range facts.Users {
			if !requested[fact.UserID] || fact.UnclassifiedOrders < 0 || (fact.HistoryComplete && fact.UnclassifiedOrders != 0) || (!fact.HistoryComplete && (fact.FirstPaidAt != nil || fact.SecondPaidAt != nil)) || (fact.SecondPaidAt != nil && (fact.FirstPaidAt == nil || *fact.SecondPaidAt < *fact.FirstPaidAt)) {
				respond(w, nil, errUnavailable)
				return
			}
			delete(requested, fact.UserID)
			u := byID[fact.UserID]
			observations["first_payment"][u.ID] = funnelObservation{RegisteredAt: u.Registered, At: fact.FirstPaidAt, Complete: fact.HistoryComplete, FirstIsExact: fact.HistoryComplete}
			observations["repeat_payment"][u.ID] = funnelObservation{RegisteredAt: u.Registered, At: fact.SecondPaidAt, Complete: fact.HistoryComplete, FirstIsExact: fact.HistoryComplete}
			if fact.FirstPaidAt != nil {
				firstDates[u.ID] = *fact.FirstPaidAt
			}
		}
		if h.s.ClientRead != nil {
			confirmations, e := h.s.ClientRead.ConfiguredClients(ctx, ids)
			if e != nil {
				respond(w, nil, errUnavailable)
				return
			}
			allowed := map[int64]bool{}
			for _, id := range ids {
				allowed[id] = true
			}
			for id, at := range confirmations {
				if !allowed[id] || at <= 0 {
					respond(w, nil, errUnavailable)
					return
				}
				observations["client_configured"][id] = funnelObservation{RegisteredAt: byID[id].Registered, At: &at}
			}
		}
	}
	report := funnelReport{Until: now, Days: days, Unavailable: []string{}, PaymentUpdated: cfg.PaymentSnapshotUpdatedAt, PaymentStatus: cfg.PaymentSnapshotStatus, Channels: []funnelChannel{}, Sources: []paymentSource{}}
	groups := map[string][]funnelAccount{}
	for _, u := range doc.Items {
		groups[u.Source] = append(groups[u.Source], u)
	}
	names := []string{}
	for name := range groups {
		names = append(names, name)
	}
	sort.Strings(names)
	missing := map[string]bool{}
	for _, name := range names {
		channel := funnelChannel{Source: name, Registrations: len(groups[name]), Stages: []funnelStage{}}
		for _, u := range groups[name] {
			if u.Registered+int64(days)*daySeconds <= now {
				channel.Mature++
			} else {
				channel.Observing++
			}
		}
		for _, stage := range stageIDs {
			values := []funnelObservation{}
			for _, u := range groups[name] {
				values = append(values, observations[stage][u.ID])
			}
			summary, e := summarizeFunnelStage(stage, now, days, values)
			if e != nil {
				respond(w, nil, e)
				return
			}
			if summary.Unknown > 0 {
				missing[stage] = true
			}
			channel.Stages = append(channel.Stages, summary)
		}
		report.Channels = append(report.Channels, channel)
	}
	for _, stage := range stageIDs {
		if missing[stage] {
			report.Unavailable = append(report.Unavailable, stage)
		}
	}
	rows, err := tx.QueryContext(ctx, `SELECT user_id,first_paid_at,source,evidence,rule,lookback_days,inferred FROM lmm_acquisition.first_payments WHERE history_conflict_at=0 AND snapshotted_at>0 AND lookback_days BETWEEN 1 AND 90 ORDER BY source,evidence,rule,lookback_days,inferred,user_id`)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	matched := map[int64]bool{}
	counts := map[paymentSource]int{}
	for rows.Next() {
		var id, at int64
		var v paymentSource
		if err = rows.Scan(&id, &at, &v.Source, &v.Evidence, &v.Rule, &v.Days, &v.Inferred); err != nil {
			break
		}
		if firstDates[id] == at && at > 0 {
			matched[id] = true
			counts[v]++
		}
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	for v, n := range counts {
		v.Accounts = n
		report.Sources = append(report.Sources, v)
	}
	sort.Slice(report.Sources, func(i, j int) bool {
		a, _ := json.Marshal(report.Sources[i])
		b, _ := json.Marshal(report.Sources[j])
		return string(a) < string(b)
	})
	if len(matched) != len(firstDates) || missing["first_payment"] {
		report.PaymentStatus = "incomplete"
	}
	if err = tx.Commit(); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, report, nil)
}
