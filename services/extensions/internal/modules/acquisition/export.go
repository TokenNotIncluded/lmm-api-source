// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"bytes"
	"context"
	"encoding/csv"
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
	"time"
	"unicode"
)

func csvCell(value string) string {
	clean := strings.TrimLeftFunc(value, func(r rune) bool { return unicode.IsSpace(r) || r == '\uFEFF' || r == '\u200B' })
	if len(clean) > 0 && strings.ContainsRune("=+-@", rune(clean[0])) {
		return "'" + value
	}
	return value
}
func (h *handlers) exportUsers(w http.ResponseWriter, r *http.Request, a actor) {
	from, e1 := strconv.ParseInt(r.URL.Query().Get("from"), 10, 64)
	to, e2 := strconv.ParseInt(r.URL.Query().Get("to"), 10, 64)
	source := r.URL.Query().Get("source")
	if e1 != nil || e2 != nil || from <= 0 || to <= from || to-from > 366*daySeconds || len(source) > 253 {
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
	var data []byte
	err = h.s.db.QueryRowContext(ctx, registrationQuery+`, exported AS (SELECT f.*,coalesce(c.source,'') AS corrected_source,coalesce(c.revision,0) AS correction_revision FROM filtered f LEFT JOIN lmm_acquisition.correction_heads c ON c.user_id=f.user_id AND c.updated_at >= $5 ORDER BY f.registered_at DESC,f.user_id DESC LIMIT 10001) SELECT json_build_object('unknown',(SELECT count(*) FROM cohort WHERE registered_at IS NULL),'items',coalesce((SELECT json_agg(exported) FROM exported),'[]'::json))`, string(raw), from, to, source, h.s.now().Unix()-AccountDays*daySeconds).Scan(&data)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	var doc struct {
		Unknown int
		Items   []struct {
			UserID       int64 `json:"user_id"`
			RegisteredAt int64 `json:"registered_at"`
			Source       string
			Evidence     string
			Corrected    string `json:"corrected_source"`
			Revision     int64  `json:"correction_revision"`
			FirstSuccess int64  `json:"first_success_at"`
		}
	}
	if json.Unmarshal(data, &doc) != nil || doc.Unknown > 0 {
		respond(w, nil, errUnavailable)
		return
	}
	if len(doc.Items) > 10000 {
		respond(w, nil, errLimit)
		return
	}
	records := [][]string{{"user_id", "registered_at_unix", "observed_source", "evidence", "manual_corrected_source", "correction_revision", "first_observed_success_at_unix", "filter_source", "registration_from_inclusive", "registration_to_exclusive"}}
	for _, row := range doc.Items {
		record := []string{strconv.FormatInt(row.UserID, 10), strconv.FormatInt(row.RegisteredAt, 10), row.Source, row.Evidence, row.Corrected, strconv.FormatInt(row.Revision, 10), strconv.FormatInt(row.FirstSuccess, 10), source, strconv.FormatInt(from, 10), strconv.FormatInt(to, 10)}
		for i, value := range record {
			record[i] = csvCell(value)
		}
		records = append(records, record)
	}
	var buffer bytes.Buffer
	writer := csv.NewWriter(&buffer)
	writer.WriteAll(records)
	if writer.Error() != nil {
		respond(w, nil, errUnavailable)
		return
	}
	noStore(w)
	w.Header().Set("Content-Type", "text/csv; charset=utf-8")
	w.Header().Set("Content-Disposition", `attachment; filename="acquisition-accounts.csv"`)
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(buffer.Bytes())
}
