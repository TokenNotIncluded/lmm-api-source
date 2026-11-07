package store

import (
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"time"
)

// Schema is a fresh-install schema, not a legacy migration. Installation is an
// explicit integration/deployment step; constructing the module runs no DDL.
//
//go:embed schema.sql
var Schema string

type Postgres struct{ db *sql.DB }

// NewPostgres takes a dedicated extension database pool opened by composition.
// Its database role must have rights ONLY on lmm_store, never Rust core tables.
func NewPostgres(db *sql.DB) (*Postgres, error) {
	if db == nil {
		return nil, ErrInvalid
	}
	return &Postgres{db}, nil
}
func (p *Postgres) Within(ctx context.Context, shopID string, fn func(Tx) error) error {
	if !Text(shopID, 128) || fn == nil {
		return ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	// Transaction-scoped database lock, not a Go mutex. Different shops proceed
	// independently; a hash collision only reduces concurrency, never correctness.
	if _, err = tx.ExecContext(ctx, "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", "lmm_store:"+shopID); err != nil {
		return err
	}
	if err = fn(&postgresTx{ctx, tx, shopID}); err != nil {
		var pg interface{ SQLState() string }
		if errors.As(err, &pg) && pg.SQLState() == "23505" {
			return ErrConflict
		}
		return err
	}
	return tx.Commit()
}

type postgresTx struct {
	ctx  context.Context
	tx   *sql.Tx
	shop string
}

func storeTable(name string) (string, error) {
	switch name {
	case "shops", "products", "variants", "orders", "commands", "stock_movements", "money_operations",
		"stock_items", "product_likes", "traffic_days", "traffic_receipts", "placements",
		"extore_flows", "extore_connections", "extore_products":
		return "lmm_store." + name, nil
	}
	return "", ErrInvalid
}
func (t *postgresTx) Get(kind, id string, out any) error {
	table, err := storeTable(kind)
	if err != nil {
		return err
	}
	var data []byte
	err = t.tx.QueryRowContext(t.ctx, "SELECT body FROM "+table+" WHERE shop_id=$1 AND id=$2", t.shop, id).Scan(&data)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrNotFound
	}
	if err != nil {
		return err
	}
	return json.Unmarshal(data, out)
}
func (t *postgresTx) Put(kind, id string, value any) error {
	table, err := storeTable(kind)
	if err != nil {
		return err
	}
	if !Text(id, 128) {
		return ErrInvalid
	}
	data, err := json.Marshal(value)
	if err != nil {
		return err
	}
	_, err = t.tx.ExecContext(t.ctx, "INSERT INTO "+table+" (shop_id,id,body) VALUES ($1,$2,$3::jsonb) ON CONFLICT (shop_id,id) DO UPDATE SET body=EXCLUDED.body", t.shop, id, string(data))
	return err
}
func (t *postgresTx) List(kind, after string, limit int) ([]json.RawMessage, error) {
	table, err := storeTable(kind)
	if err != nil {
		return nil, err
	}
	if !Page(limit) {
		return nil, ErrInvalid
	}
	rows, err := t.tx.QueryContext(t.ctx, "SELECT body FROM "+table+" WHERE shop_id=$1 AND id>$2 ORDER BY id LIMIT $3", t.shop, after, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []json.RawMessage{}
	for rows.Next() {
		var data []byte
		if err = rows.Scan(&data); err != nil {
			return nil, err
		}
		out = append(out, json.RawMessage(append([]byte(nil), data...)))
	}
	return out, rows.Err()
}

// ProductLikeStats counts currently liked rows and whether the viewer likes.
func (t *postgresTx) ProductLikeStats(productID string, userID int64) (int64, bool, error) {
	if !Text(productID, 128) || userID < 0 {
		return 0, false, ErrInvalid
	}
	var count int64
	err := t.tx.QueryRowContext(t.ctx, `SELECT COUNT(*) FROM lmm_store.product_likes WHERE shop_id=$1 AND body->>'product_id'=$2 AND (body->>'liked')::boolean`, t.shop, productID).Scan(&count)
	if err != nil {
		return 0, false, err
	}
	if userID == 0 {
		return count, false, nil
	}
	var item productLike
	switch err := t.Get("product_likes", productLikeID(productID, userID), &item); {
	case err == nil:
		return count, item.Liked && item.ProductID == productID && item.UserID == userID, nil
	case errors.Is(err, ErrNotFound):
		return count, false, nil
	default:
		return 0, false, err
	}
}

// StockItems is the indexed per-unit stock lookup used under the shop lock.
func (t *postgresTx) StockItems(variantID, state string, offset, limit int, random bool) ([]StockItem, error) {
	if !Text(variantID, 128) || offset < 0 || limit <= 0 || limit > maxStockImport {
		return nil, ErrInvalid
	}
	order := `(body->>'position')::bigint,id`
	if random {
		order = `random()`
	}
	rows, err := t.tx.QueryContext(t.ctx, `SELECT body FROM lmm_store.stock_items WHERE shop_id=$1 AND body->>'variant_id'=$2 AND body->>'state'<>'removed' AND ($3='' OR body->>'state'=$3) ORDER BY `+order+` OFFSET $4 LIMIT $5`, t.shop, variantID, state, offset, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []StockItem{}
	for rows.Next() {
		var data []byte
		var item StockItem
		if err = rows.Scan(&data); err != nil {
			return nil, err
		}
		if json.Unmarshal(data, &item) != nil {
			return nil, ErrUnavailable
		}
		out = append(out, item)
	}
	return out, rows.Err()
}

// PutAll inserts new rows in one statement; existing IDs are a conflict.
func (t *postgresTx) PutAll(kind string, ids []string, values []any) error {
	table, err := storeTable(kind)
	if err != nil {
		return err
	}
	if len(ids) == 0 || len(ids) != len(values) || len(ids) > maxStockImport {
		return ErrInvalid
	}
	type row struct {
		ID   string `json:"id"`
		Body any    `json:"body"`
	}
	batch := make([]row, len(ids))
	for i, id := range ids {
		if !Text(id, 128) {
			return ErrInvalid
		}
		batch[i] = row{id, values[i]}
	}
	data, err := json.Marshal(batch)
	if err != nil {
		return err
	}
	_, err = t.tx.ExecContext(t.ctx, "INSERT INTO "+table+" (shop_id,id,body) SELECT $1,e->>'id',e->'body' FROM jsonb_array_elements($2::jsonb) e", t.shop, string(data))
	return err
}
