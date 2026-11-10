// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"encoding/hex"
	"time"
)

func (p *Postgres) LegacyPickupToken(ctx context.Context, tokenHash string) (string, error) {
	if raw, err := hex.DecodeString(tokenHash); err != nil || len(raw) != 32 {
		return "", ErrNotFound
	}
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	rows, err := p.db.QueryContext(ctx, `SELECT body->>'token' FROM lmm_store.commands WHERE id=$1 AND body->>'kind'='legacy-store-pickup-v1' AND body->>'token_hash'=$2 LIMIT 2`, ID("legacy-store-pickup-v1", tokenHash), tokenHash)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	count := 0
	token := ""
	for rows.Next() {
		if err := rows.Scan(&token); err != nil {
			return "", err
		}
		count++
	}
	if err := rows.Err(); err != nil {
		return "", err
	}
	if count != 1 {
		return "", ErrNotFound
	}
	return token, nil
}
