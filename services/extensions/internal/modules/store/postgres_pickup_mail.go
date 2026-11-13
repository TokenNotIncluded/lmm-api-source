// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"time"
)

// PendingPickupMail reads only extension-owned commands, without decrypting or
// returning recipients/tokens. No schema or Core database access is required.
func (p *Postgres) PendingPickupMail(ctx context.Context, after PickupMailCursor, limit int) ([]PickupMailJob, error) {
	if !Page(limit) || (after != (PickupMailCursor{}) && (!Text(after.ShopID, 128) || !Text(after.ID, 128))) {
		return nil, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	rows, err := p.db.QueryContext(ctx, `SELECT shop_id,id,body->>'order_id' FROM lmm_store.commands
 WHERE body->>'kind'='store-pickup-mail-v1' AND body->>'state' IN ('pending','retry_wait','dispatching') AND (shop_id,id)>($1,$2)
 ORDER BY shop_id,id LIMIT $3`, after.ShopID, after.ID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	jobs := []PickupMailJob{}
	for rows.Next() {
		var job PickupMailJob
		if err := rows.Scan(&job.Cursor.ShopID, &job.Cursor.ID, &job.OrderID); err != nil {
			return nil, err
		}
		if job.Cursor.ID != pickupMailID(job.OrderID) || !Text(job.OrderID, 128) {
			return nil, ErrUnavailable
		}
		jobs = append(jobs, job)
	}
	return jobs, rows.Err()
}
