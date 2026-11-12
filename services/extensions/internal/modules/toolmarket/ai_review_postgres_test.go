// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"database/sql"
	"database/sql/driver"
	"errors"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
	"github.com/lib/pq"
)

// The server really commits. Only the client's acknowledgement is replaced by
// an error, preserving the business state of a lost COMMIT response without
// adding test-only hooks to production storage.
type reviewLostACKConnector struct {
	driver.Connector
	lose *atomic.Bool
}

func (c reviewLostACKConnector) Connect(ctx context.Context) (driver.Conn, error) {
	conn, err := c.Connector.Connect(ctx)
	if err != nil {
		return nil, err
	}
	return &reviewLostACKConnection{Conn: conn, lose: c.lose}, nil
}

type reviewLostACKConnection struct {
	driver.Conn
	lose *atomic.Bool
}

func (c *reviewLostACKConnection) ExecContext(ctx context.Context, query string, args []driver.NamedValue) (driver.Result, error) {
	return c.Conn.(driver.ExecerContext).ExecContext(ctx, query, args)
}
func (c *reviewLostACKConnection) QueryContext(ctx context.Context, query string, args []driver.NamedValue) (driver.Rows, error) {
	return c.Conn.(driver.QueryerContext).QueryContext(ctx, query, args)
}
func (c *reviewLostACKConnection) BeginTx(ctx context.Context, opts driver.TxOptions) (driver.Tx, error) {
	tx, err := c.Conn.(driver.ConnBeginTx).BeginTx(ctx, opts)
	if err != nil {
		return nil, err
	}
	return reviewLostACKTransaction{Tx: tx, lose: c.lose}, nil
}

type reviewLostACKTransaction struct {
	driver.Tx
	lose *atomic.Bool
}

func (tx reviewLostACKTransaction) Commit() error {
	if err := tx.Tx.Commit(); err != nil {
		return err
	}
	if tx.lose.Swap(false) {
		return errors.New("synthetic lost COMMIT acknowledgement after PostgreSQL commit")
	}
	return nil
}

func TestPostgresMarketAIReviewLostCommitACKRestoresPreviousLiveVersion(t *testing.T) {
	raw := os.Getenv("LMM_MARKET_REVIEW_TEST_DSN")
	if raw == "" {
		t.Skip("requires a fresh isolated PostgreSQL database")
	}
	u, err := url.Parse(raw)
	if err != nil || (u.Scheme != "postgres" && u.Scheme != "postgresql") || u.Hostname() != "127.0.0.1" || !strings.HasSuffix(u.Path, "_test") {
		t.Fatal("requires a local isolated test database")
	}
	connector, err := pq.NewConnector(raw)
	if err != nil {
		t.Fatal("invalid private PostgreSQL fixture")
	}
	var lose atomic.Bool
	db := sql.OpenDB(reviewLostACKConnector{Connector: connector, lose: &lose})
	defer db.Close()
	ctx := context.Background()
	var exists bool
	if err = db.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='lmm_toolmarket')`).Scan(&exists); err != nil || exists {
		t.Fatal("toolmarket fixture is not fresh")
	}
	if _, err = db.ExecContext(ctx, PostgresSchema); err != nil {
		t.Fatal(err)
	}
	key := bytes.Repeat([]byte{73}, 32)
	storage, err := InitPostgresStore(ctx, db, key)
	if err != nil {
		t.Fatal(err)
	}
	defer storage.Close()
	mock := newMock(t)
	auth := &webTestAuthority{}
	m, err := New(storage, auth, Config{WebOnly: true, CallsPerMinute: 600})
	if err != nil {
		t.Fatal(err)
	}
	useMock(t, m, mock)
	dir := t.TempDir()
	if err = os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	policy, err := marketreview.NewFilePolicy(filepath.Join(dir, "review.toml"))
	if err != nil {
		t.Fatal(err)
	}
	core := &reviewAuthority{invalidateErr: ErrUnavailable}
	engine := &marketreview.Engine{Policy: policy, Classifier: core}
	if err = m.ConfigureAIReview(engine); err != nil {
		t.Fatal(err)
	}
	s := webSaved(t, m, "1", webInput(mock.server.URL+"/mcp", "public"))
	webPublish(t, m, "1", s)
	previous := s.DraftVersionID
	core.expectedLive = previous
	input := webInput(mock.server.URL+"/mcp", "public")
	input.Name = "Revised synthetic listing"
	value, err := webRequest(t, m, "1", "save", s.ID, input)
	if err != nil {
		t.Fatal(err)
	}
	s = value.(WebServiceRecord)
	if _, err = webRequest(t, m, "1", "validate", s.ID, nil); err != nil {
		t.Fatal(err)
	}
	if err = policy.Save(ctx, marketreview.Modes{Tool: marketreview.Auto, Store: marketreview.Off}); err != nil {
		t.Fatal(err)
	}
	if _, err = webRequest(t, m, "1", "submit", s.ID, map[string]any{"version_id": s.DraftVersionID}); err != nil {
		t.Fatal(err)
	}
	core.hook = func() { lose.Store(true) }
	if err = m.WorkAIReviews(ctx, 1); !errors.Is(err, ErrUnavailable) || !core.coreActive || core.approved != 1 {
		t.Fatal("did not reach lost local COMMIT ACK", err, core)
	}
	if err = storage.view(func(d database) error {
		v := d.Web.Versions[s.DraftVersionID]
		if v.Version.Status != "published" || !v.AIReview.Jobs[0].ApprovalPending || !v.AIReview.Jobs[0].Applied || d.Web.Services[s.ID].LiveVersionID != s.DraftVersionID {
			t.Fatal("PostgreSQL did not commit the uncertain publication")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if err = storage.Close(); err != nil {
		t.Fatal(err)
	}
	// Reconstruct through a fresh connection pool; no old Store memory is used.
	readDB := sql.OpenDB(connector)
	defer readDB.Close()
	reopened, err := OpenPostgresStore(ctx, readDB, key)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	restarted, err := New(reopened, auth, Config{WebOnly: true, CallsPerMinute: 600})
	if err != nil {
		t.Fatal(err)
	}
	if err = restarted.ConfigureAIReview(engine); err != nil {
		t.Fatal(err)
	}
	useMock(t, restarted, mock)
	restarted.now = func() time.Time { return time.Now().Add(2 * time.Minute) }
	core.invalidateErr = nil
	if err = policy.Save(ctx, marketreview.Modes{Tool: marketreview.Off, Store: marketreview.Off}); err != nil {
		t.Fatal(err)
	}
	if err = restarted.WorkAIReviews(ctx, 1); err != nil {
		t.Fatal(err)
	}
	if core.coreActive || core.classified != 1 || core.approved != 1 || core.invalidated != 2 {
		t.Fatal("recovery retried provider or retained uncertain Core approval", core)
	}
	if err = reopened.view(func(d database) error {
		service := d.Web.Services[s.ID]
		v := d.Web.Versions[s.DraftVersionID]
		j := v.AIReview.Jobs[0]
		if service.LiveVersionID != previous || service.DraftVersionID != s.DraftVersionID || v.Version.Status != "pending" || j.Status != "failed" || j.Applied || j.ApprovalPending || j.Text != "" || *j.Outcome != "manual_required" {
			t.Fatal("local recovery did not restore previous publication and manual queue")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}
