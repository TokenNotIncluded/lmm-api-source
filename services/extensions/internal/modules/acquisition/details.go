// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"encoding/json"
	"net/http"
	"time"
)

func (h *handlers) userDetail(w http.ResponseWriter, r *http.Request, a actor) {
	user, err := h.detailTarget(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()
	var result json.RawMessage
	now := h.s.now().Unix()
	err = h.s.db.QueryRowContext(ctx, `WITH registration AS (SELECT coalesce($4::bigint,(SELECT registered_at FROM lmm_acquisition.cohort WHERE user_id=$1)) AS at), recent AS (SELECT v.id,v.link_id,v.source,v.medium,v.campaign,v.content,v.referrer_host,v.landing,v.evidence,v.created_at FROM lmm_acquisition.visits v JOIN lmm_acquisition.visitors b ON b.id=v.visitor_id WHERE b.user_id=$1 AND v.created_at >= $3 ORDER BY v.created_at DESC,v.id DESC LIMIT 20) SELECT json_build_object('user_id',$1::bigint,'registered_at',registration.at,'historical',registration.at < (SELECT started_at FROM lmm_acquisition.config WHERE singleton),'self_reported',(SELECT json_build_object('source',source,'detail',detail,'updated_at',updated_at) FROM lmm_acquisition.self_reports WHERE user_id=$1 AND updated_at >= $2),'attribution',(SELECT row_to_json(a) FROM lmm_acquisition.accounts a WHERE user_id=$1 AND created_at >= $2),'first_success_at',(SELECT coalesce(min(first_at),0) FROM lmm_acquisition.activities WHERE user_id=$1),'recent',coalesce((SELECT json_agg(recent) FROM recent),'[]'::json),'recent_limit',20) FROM registration`, user.UserID, now-AccountDays*daySeconds, now-RawDays*daySeconds, user.RegisteredAt).Scan(&result)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, result, nil)
}
