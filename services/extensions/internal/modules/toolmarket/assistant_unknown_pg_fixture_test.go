// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

// Separate DSN files keep this fresh-database case independent of other PostgreSQL tests.
const assistantLostRuntimeDSN = "LMM_ASSISTANT_LOST_RESPONSE_TEST_DSN_FILE"

// AssistantLostResponseFixture provides real module storage and a synthetic
// Core credit authority to the external assistant_market adapter test.
type AssistantLostResponseFixture struct {
	Module    *Module
	Quote     LegacyCreditQuote
	authority *assistantExecutionAuthority
	configure func(*Module)
	db        *sql.DB
	key       []byte
	calls     atomic.Int32
	pgPID     int
}

func NewAssistantLostResponseFixture(t *testing.T) *AssistantLostResponseFixture {
	t.Helper()
	ctx := context.Background()
	db := toolmarketDatabaseFile(t, assistantLostRuntimeDSN)
	owner := toolmarketDatabaseFile(t, "LMM_ASSISTANT_LOST_RESPONSE_TEST_OWNER_DSN_FILE")
	role := toolmarketFixtureRuntimeRole(t, owner, db)
	// The production installer refuses an existing schema. Use a fresh database per run.
	if _, err := owner.ExecContext(ctx, PostgresSchema); err != nil {
		t.Fatal(err)
	}
	f := &AssistantLostResponseFixture{db: db, key: bytes.Repeat([]byte{71}, 32)}
	seed, a, _, _, _ := assistantExecutionFixture(t, false)
	f.authority = a
	v := seed.store.data.Web.Versions["version"]
	v.ValidationDigest, v.Version.Digest = v.Digest, v.Digest
	v.Tools[0].PriceQuota = 7
	seed.store.data.Web.Versions["version"] = v
	quote, err := seed.LegacyReadToolQuote(ctx, "1", "1", "service", "version", "tool")
	if err != nil {
		t.Fatal(err)
	}
	f.Quote = quote
	a.r.QuoteID, a.r.PricingDigest, a.r.Held = quote.ID, quote.PricingDigest, 7
	a.witnessAvailable = true
	g := &a.scope.Consents[0]
	g.QuoteID, g.RulesHash, g.FrozenPriceQuota, g.MaxPriceQuota, g.MaxTotalQuota = quote.ID, quote.PricingDigest, 7, 7, 7

	ownerStore, err := InitPostgresStore(ctx, owner, f.key)
	if err != nil {
		t.Fatal(err)
	}
	if err = ownerStore.update(func(d *database) error { *d = seed.store.data; return nil }); err != nil {
		t.Fatal(err)
	}
	if err = ownerStore.Close(); err != nil {
		t.Fatal(err)
	}
	role = `"` + strings.ReplaceAll(role, `"`, `""`) + `"`
	if _, err = owner.ExecContext(ctx, `GRANT USAGE ON SCHEMA lmm_toolmarket TO `+role+`; GRANT SELECT,UPDATE ON lmm_toolmarket.snapshot TO `+role); err != nil {
		t.Fatal(err)
	}

	// Close the TCP connection *after* the remote tools/call side effect and
	// before writing a single byte of the HTTP response.
	peer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var rpc struct {
			Method string
			Params struct{ Name string }
		}
		if err := json.NewDecoder(r.Body).Decode(&rpc); err != nil || rpc.Method != "tools/call" || rpc.Params.Name != "echo" {
			t.Errorf("unexpected supplier request: %v %s %s", err, rpc.Method, rpc.Params.Name)
			http.Error(w, "unexpected request", 400)
			return
		}
		f.calls.Add(1)
		c, _, err := w.(http.Hijacker).Hijack()
		if err != nil {
			t.Error(err)
			return
		}
		c.Close()
	}))
	t.Cleanup(peer.Close)
	local, err := url.Parse(peer.URL)
	if err != nil {
		t.Fatal(err)
	}
	disco := seed.net.client.Transport
	transport := legacyProviderTransport(func(req *http.Request) (*http.Response, error) {
		body, err := io.ReadAll(req.Body)
		req.Body.Close()
		if err != nil {
			return nil, err
		}
		var rpc struct{ Method string }
		if err = json.Unmarshal(body, &rpc); err != nil {
			return nil, err
		}
		copy := req.Clone(req.Context())
		copy.Body = io.NopCloser(bytes.NewReader(body))
		if rpc.Method != "tools/call" {
			return disco.RoundTrip(copy)
		}
		copy.URL.Scheme, copy.URL.Host = local.Scheme, local.Host
		copy.Host = local.Host
		return peer.Client().Transport.RoundTrip(copy)
	})
	f.configure = func(m *Module) { m.now = seed.now; m.net.client = &http.Client{Transport: transport} }
	f.open(t)
	t.Cleanup(func() {
		if err := f.Module.store.Close(); err != nil {
			t.Error(err)
		}
	})
	return f
}

func (f *AssistantLostResponseFixture) open(t *testing.T) {
	t.Helper()
	var pid int
	if err := f.db.QueryRow("SELECT pg_backend_pid()").Scan(&pid); err != nil {
		t.Fatal(err)
	}
	if pid == f.pgPID {
		t.Fatal("reopen reused the prior PostgreSQL connection")
	}
	store, err := OpenPostgresStore(context.Background(), f.db, f.key)
	if err != nil {
		t.Fatal(err)
	}
	m, err := New(store, f.authority, Config{WebOnly: true})
	if err != nil {
		t.Fatal(err)
	}
	f.configure(m)
	f.Module, f.pgPID = m, pid
	t.Logf("opened module through PostgreSQL backend PID %d", pid)
}

func (f *AssistantLostResponseFixture) Reopen(t *testing.T) {
	t.Helper()
	if err := f.Module.store.Close(); err != nil {
		t.Fatal(err)
	}
	if err := f.db.Close(); err != nil {
		t.Fatal(err)
	}
	f.db = toolmarketDatabaseFile(t, assistantLostRuntimeDSN)
	f.open(t)
	// Exercise recovery after the ordinary resolution deadline.
	f.Module.now = func() time.Time { return time.Unix(500, 0) }
}

func (f *AssistantLostResponseFixture) CheckUnknown(t *testing.T) {
	t.Helper()
	const op = "original-assistant-op"
	ctx := context.Background()
	r, err := f.Module.LegacyGetExecutionByRequestKey(ctx, "1", "1", "builtin-assistant", op)
	if !errors.Is(err, ErrPending) || r.State != "unknown" || r.AssistantToolOperationID != op || r.Input.RequestKey != op || r.Core.State != "unknown" || r.Core.ID != "call" || r.Core.DispatchID != "dispatch" || r.Core.Held != 7 || r.ResultDigest != "" || len(r.RawResult) != 0 || r.Success {
		t.Fatalf("missing original unknown record: state=%q core=%q held=%d result=%q error=%v", r.State, r.Core.State, r.Core.Held, r.ResultDigest, err)
	}
	a := f.authority
	if f.calls.Load() != 1 || a.assistantReserves != 1 || a.reserves != 0 || a.dispatches != 1 || a.unknowns != 1 || a.finishes != 0 || a.releases != 0 {
		t.Fatalf("supplier=%d assistant_reserves=%d ordinary_reserves=%d dispatches=%d unknowns=%d finishes=%d releases=%d", f.calls.Load(), a.assistantReserves, a.reserves, a.dispatches, a.unknowns, a.finishes, a.releases)
	}
	t.Logf("state=%s supplier_calls=%d assistant_holds=%d dispatches=%d settlement_calls=%d releases=%d synthetic_held=%d", r.State, f.calls.Load(), a.assistantReserves, a.dispatches, a.finishes, a.releases, r.Core.Held)
}
