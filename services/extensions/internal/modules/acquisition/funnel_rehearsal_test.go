// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// Opt-in restored-data self-test. All three databases must be local, restored,
// and read-only. Authentication is synthetic; Core identity/payment reads are real.
func TestRestoredFunnelLive(t *testing.T) {
	socket := os.Getenv("LMM_TEST_CORE_FUNNEL_SOCKET")
	if socket == "" {
		t.Skip("requires the explicitly restored Core rehearsal")
	}
	open := func(variable, prefix string) *sql.DB {
		dsn := os.Getenv(variable)
		if dsn == "" {
			t.Fatalf("%s required", variable)
		}
		db, err := sql.Open("postgres", dsn)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { db.Close() })
		var local bool
		if err = db.QueryRow(`SELECT inet_server_addr() IS NULL AND current_setting('listen_addresses')='' AND current_setting('default_transaction_read_only')='on' AND current_database() LIKE $1`, prefix+"%").Scan(&local); err != nil || !local {
			t.Fatalf("read-only isolated rehearsal required: %v", err)
		}
		return db
	}
	db := open("LMM_REHEARSAL_ACQUISITION_READ_DSN", "lmm_acquisition_audit_full_")
	cdb := open("LMM_REHEARSAL_COMMUNITY_READ_DSN", "lmm_community_audit_full_")
	client, err := community.NewCoreRead(socket, []byte("synthetic-community-purpose-token-20261104"))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	service, err := New(context.Background(), db, "https://synthetic.example", true)
	if err != nil {
		t.Fatal(err)
	}
	c, err := community.New(context.Background(), cdb, "https://synthetic.example")
	if err != nil {
		t.Fatal(err)
	}
	service.UserRead = client
	service.ClientRead = c
	now := time.Now().Unix()
	expected := 0
	eligibleIDs := []int64{}
	earliest := now
	cursor := int64(0)
	for {
		page, e := client.UserPage(context.Background(), cursor, 100)
		if e != nil {
			t.Fatal(e)
		}
		for _, u := range page.Users {
			if u.PlatformRole == "user" {
				if u.RegisteredAt == nil {
					t.Fatal("unknown registration time")
				}
				if *u.RegisteredAt >= 1 && *u.RegisteredAt < now {
					expected++
					eligibleIDs = append(eligibleIDs, u.UserID)
					earliest = min(earliest, *u.RegisteredAt)
				}
			}
		}
		if !page.HasMore {
			break
		}
		cursor = page.NextID
	}
	mux := http.NewServeMux()
	service.Register(mux, listVerifier{5})

	total := 0
	reports := []json.RawMessage{}
	for from := earliest; from < now; {
		to := min(now, from+366*daySeconds)
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/funnel?from=%d&to=%d&observation_days=30", from, to), nil))
		var v struct {
			Success bool
			Data    funnelReport
		}
		if json.Unmarshal(w.Body.Bytes(), &v) != nil || !v.Success || w.Code != 200 {
			t.Fatalf("restored funnel failed: HTTP %d", w.Code)
		}
		for _, c := range v.Data.Channels {
			total += c.Registrations
			for _, stage := range c.Stages {
				if stage.Observed+stage.NotObserved+stage.Unknown != c.Registrations {
					t.Fatalf("stage total mismatch: %s", stage.ID)
				}
			}
		}
		rw := httptest.NewRecorder()
		mux.ServeHTTP(rw, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/report?from=%d&to=%d", from, to), nil))
		var report struct {
			Success bool
			Data    acquisitionReport
		}
		if json.Unmarshal(rw.Body.Bytes(), &report) != nil || !report.Success || rw.Code != 200 {
			t.Fatalf("restored report failed: HTTP %d", rw.Code)
		}
		registered := 0
		for _, c := range report.Data.Channels {
			registered += c.Registrations
		}
		scopeTotal := 0
		for _, c := range v.Data.Channels {
			scopeTotal += c.Registrations
		}
		if registered != scopeTotal {
			t.Fatal("report/funnel cohort counts disagree")
		}

		amounts := map[string]community.CashCurrency{}
		var unclassified int64
		for start := 0; start < len(eligibleIDs); start += 100 {
			facts, e := client.CashPaymentTotals(context.Background(), eligibleIDs[start:min(start+100, len(eligibleIDs))], from, to)
			if e != nil {
				t.Fatal(e)
			}
			for _, u := range facts.Users {
				unclassified += u.UnclassifiedOrders
				for _, c := range u.Currencies {
					v := amounts[c.Currency]
					v.Currency = c.Currency
					v.PaidMicros += c.PaidMicros
					v.RefundMicros += c.RefundMicros
					v.NetMicros += c.NetMicros
					v.Orders++
					amounts[c.Currency] = v
				}
			}
		}
		actual := map[string]community.CashCurrency{}
		for _, p := range report.Data.Payments {
			if p.Net != p.Paid-p.Refund || p.Payers <= 0 {
				t.Fatal("invalid report money totals")
			}
			v := actual[p.Currency]
			v.Currency = p.Currency
			v.PaidMicros += p.Paid
			v.RefundMicros += p.Refund
			v.NetMicros += p.Net
			v.Orders += int64(p.Payers)
			actual[p.Currency] = v
		}
		wantJSON, _ := json.Marshal(amounts)
		gotJSON, _ := json.Marshal(actual)
		if string(wantJSON) != string(gotJSON) || unclassified != report.Data.Unclassified {
			t.Fatal("report amounts/refunds/payers/unknowns disagree with real nonstaff Core facts")
		}

		reports = append(reports, append(json.RawMessage(nil), w.Body.Bytes()...))
		from = to
	}
	if expected == 0 || total != expected {
		t.Fatalf("live role/cohort mismatch: expected %d got %d", expected, total)
	}
	output := os.Getenv("LMM_REHEARSAL_FUNNEL_REPORT")
	if output != "" {
		base, e := filepath.Abs(filepath.Join(os.Getenv("HOME"), "Backups"))
		if e != nil {
			t.Fatal(e)
		}
		relative, e := filepath.Rel(base, output)
		if e != nil || !filepath.IsLocal(relative) {
			t.Fatal("report must stay under private Backups")
		}
		data, e := json.Marshal(reports)
		if e != nil {
			t.Fatal(e)
		}
		if err = os.WriteFile(output, data, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	t.Logf("restored live Core/Go funnel: %d nonstaff accounts; %d bounded windows", total, len(reports))
}
