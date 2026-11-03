// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"github.com/lib/pq"
)

// ConfiguredClients returns retained confirmations, not inferred installation
// times. Missing confirmations do not prove that a client was never configured.
func (s *Service) ConfiguredClients(ctx context.Context, ids []int64) (map[int64]int64, error) {
	if len(ids) == 0 || len(ids) > 100 {
		return nil, errInvalid
	}
	for _, id := range ids {
		if id <= 0 {
			return nil, errInvalid
		}
	}
	rows, err := s.db.QueryContext(ctx, `SELECT user_id,client_configured_at FROM lmm_community.onboarding_todos WHERE user_id=ANY($1) AND client_configured_at>0`, pq.Array(ids))
	if err != nil {
		return nil, errUnavailable
	}
	defer rows.Close()
	result := map[int64]int64{}
	for rows.Next() {
		var id, at int64
		if err = rows.Scan(&id, &at); err != nil {
			return nil, errUnavailable
		}
		result[id] = at
	}
	if rows.Err() != nil {
		return nil, errUnavailable
	}
	return result, nil
}
