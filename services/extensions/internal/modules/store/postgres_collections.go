package store

import (
	"context"
	"database/sql"
	"time"
)

func (p *Postgres) readCollection(ctx context.Context, userID int64, cart bool, offset, limit int) ([]collectionReference, error) {
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	query := `SELECT '',product_id,'',0,created_at,created_at FROM lmm_store.favorites WHERE user_id=$1 ORDER BY created_at DESC,product_id OFFSET $2 LIMIT $3`
	if cart {
		query = `SELECT id,product_id,variant_id,quantity,created_at,updated_at FROM lmm_store.cart_items WHERE user_id=$1 ORDER BY updated_at DESC,id OFFSET $2 LIMIT $3`
	}
	rows, err := p.db.QueryContext(ctx, query, userID, offset, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []collectionReference{}
	for rows.Next() {
		var ref collectionReference
		if err := rows.Scan(&ref.ID, &ref.ProductID, &ref.VariantID, &ref.Quantity, &ref.CreatedAt, &ref.UpdatedAt); err != nil {
			return nil, err
		}
		result = append(result, ref)
	}
	return result, rows.Err()
}

func (p *Postgres) putCollection(ctx context.Context, userID int64, cart bool, ref collectionReference) (collectionReference, error) {
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	now := time.Now().UTC().Unix()
	var err error
	if cart {
		err = p.db.QueryRowContext(ctx, `INSERT INTO lmm_store.cart_items(user_id,id,product_id,variant_id,quantity,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$6)
ON CONFLICT(user_id,product_id,variant_id) DO UPDATE SET quantity=EXCLUDED.quantity,updated_at=EXCLUDED.updated_at
RETURNING id,product_id,variant_id,quantity,created_at,updated_at`, userID, ref.ID, ref.ProductID, ref.VariantID, ref.Quantity, now).Scan(&ref.ID, &ref.ProductID, &ref.VariantID, &ref.Quantity, &ref.CreatedAt, &ref.UpdatedAt)
	} else {
		err = p.db.QueryRowContext(ctx, `INSERT INTO lmm_store.favorites(user_id,product_id,created_at) VALUES($1,$2,$3)
ON CONFLICT(user_id,product_id) DO UPDATE SET product_id=EXCLUDED.product_id RETURNING created_at`, userID, ref.ProductID, now).Scan(&ref.CreatedAt)
	}
	return ref, err
}

func (p *Postgres) deleteCollection(ctx context.Context, userID int64, cart bool, id string) (int64, error) {
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	query := `DELETE FROM lmm_store.favorites WHERE user_id=$1 AND ($2='' OR product_id=$2)`
	if cart {
		query = `DELETE FROM lmm_store.cart_items WHERE user_id=$1 AND ($2='' OR id=$2)`
	}
	result, err := p.db.ExecContext(ctx, query, userID, id)
	if err != nil {
		return 0, err
	}
	return result.RowsAffected()
}

func (p *Postgres) cleanupCollections(ctx context.Context, userID int64) (int64, int64, error) {
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead})
	if err != nil {
		return 0, 0, err
	}
	defer tx.Rollback()
	metadata := `(CASE WHEN jsonb_typeof(p.body->'legacy')='object' THEN p.body->'legacy' ELSE p.body->'legacy_public' END)`
	visibility := `COALESCE(NULLIF(` + metadata + `->>'visibility',''),CASE WHEN COALESCE((` + metadata + `->>'test_mode')::boolean,false) THEN 'private' ELSE 'public' END)`
	status := `COALESCE(` + metadata + `->>'status',CASE WHEN (p.body->>'published')::boolean THEN 'published' ELSE 'draft' END)`
	retained := `SELECT p.id FROM lmm_store.products p JOIN lmm_store.shops s ON s.shop_id=p.shop_id AND s.id=p.shop_id
WHERE p.id=c.product_id AND s.body->'owner'->>'kind'='personal' AND
((` + visibility + ` IN ('public','registered') AND ` + status + ` IN ('published','paused') AND (p.body->'legacy' IS NULL OR COALESCE((p.body->'legacy'->>'approved')::boolean,false))) OR
(` + visibility + `='private' AND (s.body->'owner'->>'id')::bigint=$1 AND ` + status + ` IN ('draft','pending','published','paused')))`
	cart, err := tx.ExecContext(ctx, `DELETE FROM lmm_store.cart_items c WHERE c.user_id=$1 AND
(NOT EXISTS (`+retained+`) OR NOT EXISTS (SELECT 1 FROM lmm_store.variants v WHERE v.id=c.variant_id AND v.body->>'product_id'=c.product_id AND (v.body->'legacy' IS NULL OR COALESCE((v.body->'legacy'->>'enabled')::boolean,false))))`, userID)
	if err != nil {
		return 0, 0, err
	}
	favorites, err := tx.ExecContext(ctx, `DELETE FROM lmm_store.favorites c WHERE c.user_id=$1 AND NOT EXISTS (`+retained+`)`, userID)
	if err != nil {
		return 0, 0, err
	}
	cartCount, err := cart.RowsAffected()
	if err != nil {
		return 0, 0, err
	}
	favoriteCount, err := favorites.RowsAffected()
	if err != nil {
		return 0, 0, err
	}
	if err := tx.Commit(); err != nil {
		return 0, 0, err
	}
	return cartCount, favoriteCount, nil
}
