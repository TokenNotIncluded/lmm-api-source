// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package freshinstall

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
)

// This fixture is a private, newly provisioned PG cluster, never an application
// configuration or an operator's normal owner credential.
func TestCheckinHistoryCLIActualPostgres(t *testing.T) {
	path := os.Getenv("LMM_CHECKINS_HISTORY_CLI_FIXTURE")
	if path == "" {
		t.Skip("requires a private synthetic history CLI fixture")
	}
	var fixture struct {
		OwnerURLFile        string `json:"owner_url_file"`
		RuntimeURLFile      string `json:"runtime_url_file"`
		AdminURLFile        string `json:"admin_url_file"`
		FreshOwnerURLFile   string `json:"fresh_owner_url_file"`
		FreshRuntimeURLFile string `json:"fresh_runtime_url_file"`
	}
	raw, e := os.ReadFile(path)
	if e != nil || json.Unmarshal(raw, &fixture) != nil {
		t.Fatal("invalid fixture")
	}
	open := func(file string) *sql.DB {
		t.Helper()
		raw, e := os.ReadFile(file)
		if e != nil {
			t.Fatal(e)
		}
		db, e := sql.Open("postgres", strings.TrimSpace(string(raw)))
		if e != nil {
			t.Fatal(e)
		}
		db.SetMaxOpenConns(1)
		t.Cleanup(func() { db.Close() })
		var marker string
		if e = db.QueryRow("SELECT current_setting('lmm.synthetic',true)").Scan(&marker); e != nil || marker != "checkins-extension-test" {
			t.Fatal("not synthetic storage")
		}
		return db
	}
	admin := open(fixture.AdminURLFile)
	owner := open(fixture.OwnerURLFile)
	if _, e = owner.Exec("SET ROLE checkins_cli_owner"); e != nil {
		t.Fatal(e)
	}
	if _, e = owner.Exec(checkins.Schema + "\n" + checkins.SettingsSchema); e != nil {
		t.Fatal(e)
	}
	owner.Close()
	hash := func(raw []byte) string { h := sha256.Sum256(raw); return hex.EncodeToString(h[:]) }
	input := checkins.LegacyHistoryInput{
		Scope: checkins.LegacyHistoryScope, SourceNamespace: "synthetic.checkin.cli",
		SourceArchiveSHA256: strings.Repeat("a", 64), SourceExportSHA256: strings.Repeat("b", 64), IdentityReceiptSHA256: strings.Repeat("c", 64),
		CalendarTimezone: "Asia/Shanghai", CalendarOffsetSeconds: checkins.Offset, ObservedAt: 1893542400, SourceRowCount: 2,
		SourceUserIDs: []int64{1}, Mapping: []checkins.LegacyHistoryOwner{{SourceUserID: 1, TargetUserID: 501, TargetPersonalAccountID: 733}},
		Rows: []checkins.LegacyHistoryRecord{{ID: 10, UserID: 1, CheckinDate: "2030-01-01", QuotaAwarded: 42, CreatedAt: 1893456000}, {ID: 11, UserID: 999, CheckinDate: "2030-01-01", QuotaAwarded: 0, CreatedAt: 1893456000}},
	}
	mapRaw, _ := json.Marshal(input.Mapping)
	input.IdentityMappingSHA256 = hash(mapRaw)
	raw, _ = json.Marshal(input)
	inputFile := filepath.Join(t.TempDir(), "history.json")
	if e = os.WriteFile(inputFile, raw, 0600); e != nil {
		t.Fatal(e)
	}
	args := []string{"--owner-url-file", fixture.OwnerURLFile, "--database", "lmm_checkins_history_cli", "--owner-role", "checkins_cli_owner", "--runtime-role", "checkins_cli_runtime", "--input-file", inputFile, "--input-sha256", hash(raw), "--operation-id", "synthetic-history-cli-operation"}
	var output bytes.Buffer
	ctx := context.Background()
	if e = RunCheckinHistoryCLI(ctx, append(append([]string{}, args...), "--recover"), &output); !errors.Is(e, checkins.ErrConflict) {
		t.Fatal("missing receipt was fabricated", e)
	}
	if output.Len() != 0 {
		t.Fatal("failed recovery emitted success")
	}
	if e = RunCheckinHistoryCLI(ctx, args, &output); e != nil {
		t.Fatal("CLI apply", e)
	}
	type result struct {
		Database       string                        `json:"database"`
		HistoryVersion int                           `json:"history_version"`
		Receipt        checkins.LegacyHistoryReceipt `json:"receipt"`
		Recovered      bool                          `json:"recovered"`
		BalanceEffect  string                        `json:"balance_effect"`
	}
	var original result
	if json.Unmarshal(output.Bytes(), &original) != nil || original.Database != "lmm_checkins_history_cli" || original.HistoryVersion != 1 || original.Recovered || original.BalanceEffect != "none" || original.Receipt.MappedRows != 1 || original.Receipt.ArchivedRows != 1 || original.Receipt.AddedRows != 2 {
		t.Fatal("CLI result contract", output.String())
	}
	for _, recoverOnly := range []bool{true, false} {
		output.Reset()
		next := append([]string{}, args...)
		if recoverOnly {
			next = append(next, "--recover")
		}
		if e = RunCheckinHistoryCLI(ctx, next, &output); e != nil {
			t.Fatal("original CLI receipt", e)
		}
		var readback result
		if json.Unmarshal(output.Bytes(), &readback) != nil || readback.Receipt != original.Receipt || readback.Recovered != recoverOnly {
			t.Fatal("CLI receipt changed", output.String())
		}
	}
	if e = os.Chmod(inputFile, 0644); e != nil {
		t.Fatal(e)
	}
	if e = RunCheckinHistoryCLI(ctx, args, &bytes.Buffer{}); !errors.Is(e, ErrRejected) {
		t.Fatal("public history input accepted", e)
	}
	if e = os.Chmod(inputFile, 0600); e != nil {
		t.Fatal(e)
	}
	if _, e = admin.Exec("ALTER ROLE checkins_cli_runtime LOGIN"); e != nil {
		t.Fatal(e)
	}
	if e = RunCheckinHistoryCLI(ctx, args, &bytes.Buffer{}); !errors.Is(e, checkins.ErrUnavailable) {
		t.Fatal("live runtime accepted", e)
	}
	// A fresh installation includes the same compiled addon with runtime SELECT
	// on public history, and no permission on private input/identity receipts.
	cfg := Config{Component: "checkins", OwnerURLFile: fixture.FreshOwnerURLFile, ExpectedDatabase: "lmm_checkins_history_fresh", OwnerRole: "checkins_fresh_owner", RuntimeRole: "checkins_fresh_runtime"}
	if _, e = Install(ctx, cfg); e != nil {
		t.Fatal("fresh compiled history install", e)
	}
	runtime := open(fixture.FreshRuntimeURLFile)
	var version, count int
	if e = runtime.QueryRow("SELECT version,(SELECT count(*) FROM lmm_checkins.legacy_history) FROM lmm_checkins.legacy_history_contract WHERE singleton").Scan(&version, &count); e != nil || version != 1 || count != 0 {
		t.Fatal("fresh history contract", version, count, e)
	}
	_, e = runtime.Exec("SELECT manifest FROM lmm_checkins.legacy_history_imports")
	denied(t, e)
	_, e = runtime.Exec("INSERT INTO lmm_checkins.legacy_history_contract VALUES(false,1)")
	denied(t, e)
}
