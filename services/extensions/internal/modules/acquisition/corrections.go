// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"strconv"
	"strings"
	"unicode/utf8"
)

func (h *handlers) correctionTarget(r *http.Request) (int64, error) {
	user, err := h.detailTarget(r)
	return user.UserID, err
}
func (h *handlers) detailTarget(r *http.Request) (community.CoreUserInfo, error) {
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil || id <= 0 {
		return community.CoreUserInfo{}, ErrInvalid
	}
	if h.s.UserRead == nil {
		return community.CoreUserInfo{}, errUnavailable
	}
	users, err := h.s.UserRead.Users(r.Context(), []int64{id})
	if err != nil {
		return community.CoreUserInfo{}, errUnavailable
	}
	user, ok := users[id]
	if !ok {
		return community.CoreUserInfo{}, errNotFound
	}
	if user.UserID != id {
		return community.CoreUserInfo{}, errUnavailable
	}
	if !user.Active {
		return community.CoreUserInfo{}, errNotFound
	}
	switch user.PlatformRole {
	case "user":
		return user, nil
	case "admin", "superadmin":
		return community.CoreUserInfo{}, errForbidden
	default:
		return community.CoreUserInfo{}, errUnavailable
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

func (h *handlers) saveCorrection(w http.ResponseWriter, r *http.Request, a actor) {
	id, err := h.correctionTarget(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	var input struct {
		ExpectedRevision *int64 `json:"expected_revision"`
		Source           string `json:"source"`
		Reason           string `json:"reason"`
	}
	if decode(w, r, &input, false) != nil {
		respond(w, nil, ErrInvalid)
		return
	}
	input.Reason = strings.TrimSpace(input.Reason)
	if input.ExpectedRevision == nil || *input.ExpectedRevision < 0 || input.Source == "" || Label(input.Source) != input.Source || utf8.RuneCountInString(input.Reason) < 3 || utf8.RuneCountInString(input.Reason) > 300 || strings.ContainsAny(input.Reason, "\r\n\x00@") || selfReportSecret.MatchString(input.Reason) {
		respond(w, nil, ErrInvalid)
		return
	}
	var result json.RawMessage
	err = h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		head, err := lockCorrection(ctx, tx, id)
		if err != nil {
			return err
		}
		var refused bool
		if err = tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM lmm_acquisition.consents WHERE user_id=$1 AND NOT allowed)`, id).Scan(&refused); err != nil {
			return err
		}
		if refused {
			return ErrInvalid
		}
		if head.Revision != *input.ExpectedRevision {
			return errConflict
		}
		previous := head.Source
		now := h.s.now().Unix()
		if head.Revision == 0 {
			if err = tx.QueryRowContext(ctx, `SELECT coalesce((SELECT registration_source FROM lmm_acquisition.accounts WHERE user_id=$1 AND created_at >= $2),(SELECT 'historical_unrecorded' FROM lmm_acquisition.cohort c,lmm_acquisition.config f WHERE c.user_id=$1 AND c.registered_at<f.started_at),'unknown')`, id, now-AccountDays*daySeconds).Scan(&previous); err != nil {
				return err
			}
		}
		var revision int64
		if err = tx.QueryRowContext(ctx, `INSERT INTO lmm_acquisition.corrections(user_id,previous_revision,previous_source,source,reason,actor_id,created_at) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING id`, id, head.Revision, previous, input.Source, input.Reason, a.id, now).Scan(&revision); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `UPDATE lmm_acquisition.correction_heads SET revision=$2,source=$3,updated_at=$4 WHERE user_id=$1`, id, revision, input.Source, now); err != nil {
			return err
		}
		return tx.QueryRowContext(ctx, `SELECT row_to_json(c) FROM lmm_acquisition.corrections c WHERE id=$1`, revision).Scan(&result)
	})
	respond(w, result, err)
}
