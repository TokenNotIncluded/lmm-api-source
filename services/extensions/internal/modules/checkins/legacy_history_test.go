// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package checkins

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"strings"
	"testing"
	"time"
)

func historyFixture() LegacyHistoryInput {
	at := time.Date(2030, 1, 2, 0, 0, 0, 0, time.UTC).Unix()
	x := LegacyHistoryInput{
		Scope: LegacyHistoryScope, SourceNamespace: "synthetic.checkins", SourceArchiveSHA256: strings.Repeat("a", 64),
		SourceExportSHA256: strings.Repeat("b", 64), IdentityReceiptSHA256: strings.Repeat("c", 64),
		CalendarTimezone: "Asia/Shanghai", CalendarOffsetSeconds: Offset, ObservedAt: at,
		SourceUserIDs: []int64{1, 2}, Mapping: []LegacyHistoryOwner{{SourceUserID: 1, TargetUserID: 501, TargetPersonalAccountID: 733}, {SourceUserID: 2, TargetUserID: 502, TargetPersonalAccountID: 990}},
		Rows: []LegacyHistoryRecord{
			{ID: 10, UserID: 1, CheckinDate: "2029-12-31", QuotaAwarded: 25, CreatedAt: at - 2*86400},
			{ID: 11, UserID: 1, CheckinDate: "2030-01-02", QuotaAwarded: 100, CreatedAt: at - 1},
			{ID: 12, UserID: 99, CheckinDate: "2030-01-02", QuotaAwarded: 7, CreatedAt: at - 1},
			{ID: 13, UserID: 2, CheckinDate: "2030-01-01", QuotaAwarded: 0, CreatedAt: at - 86400},
		},
	}
	x.SourceRowCount = int64(len(x.Rows))
	mapping, _ := json.Marshal(x.Mapping)
	x.IdentityMappingSHA256 = digestHistory(mapping)
	return x
}

func historyBytes(x LegacyHistoryInput) ([]byte, string) {
	raw, _ := json.Marshal(x)
	return raw, digestHistory(raw)
}

func TestLegacyHistoryInputBindingsAndExactFacts(t *testing.T) {
	x := historyFixture()
	raw, hash := historyBytes(x)
	_, rows, e := decodeLegacyHistory(raw, hash)
	if e != nil || len(rows) != 4 || rows[2].TargetUserID != nil || *rows[0].TargetUserID != 501 || *rows[0].TargetPersonalAccountID != 733 {
		t.Fatal("source owners or archive-only orphan changed", e)
	}
	if _, _, e = decodeLegacyHistory(raw, strings.Repeat("d", 64)); e != ErrInvalid {
		t.Fatal("unreviewed input accepted", e)
	}
	cases := map[string]func(*LegacyHistoryInput){
		"partial map":        func(x *LegacyHistoryInput) { x.Mapping = x.Mapping[:1] },
		"wrong mapping hash": func(x *LegacyHistoryInput) { x.IdentityMappingSHA256 = strings.Repeat("e", 64) },
		"duplicate target account": func(x *LegacyHistoryInput) {
			x.Mapping[1].TargetPersonalAccountID = x.Mapping[0].TargetPersonalAccountID
			m, _ := json.Marshal(x.Mapping)
			x.IdentityMappingSHA256 = digestHistory(m)
		},
		"duplicate source user": func(x *LegacyHistoryInput) { x.SourceUserIDs[1] = 1 },
		"incomplete history":    func(x *LegacyHistoryInput) { x.SourceRowCount++ },
		"duplicate owner day":   func(x *LegacyHistoryInput) { x.Rows[3].UserID = 1; x.Rows[3].CheckinDate = x.Rows[1].CheckinDate },
		"duplicate source ID":   func(x *LegacyHistoryInput) { x.Rows[1].ID = x.Rows[0].ID },
		"negative award":        func(x *LegacyHistoryInput) { x.Rows[0].QuotaAwarded = -1 },
		"invalid date":          func(x *LegacyHistoryInput) { x.Rows[0].CheckinDate = "2030-02-30" },
		"wrong calendar":        func(x *LegacyHistoryInput) { x.CalendarOffsetSeconds = 0 },
		"unbound timezone":      func(x *LegacyHistoryInput) { x.CalendarTimezone = "UTC" },
		"future creation":       func(x *LegacyHistoryInput) { x.Rows[0].CreatedAt = x.ObservedAt + 1 },
		"no committed receipt":  func(x *LegacyHistoryInput) { x.IdentityReceiptSHA256 = "" },
	}
	for name, change := range cases {
		t.Run(name, func(t *testing.T) {
			bad := historyFixture()
			change(&bad)
			b, h := historyBytes(bad)
			if _, _, e := decodeLegacyHistory(b, h); e != ErrInvalid {
				t.Fatal("invalid input accepted", e)
			}
		})
	}
}

func TestLegacyHistoryRealPostgresNoRewardReplay(t *testing.T) {
	ctx := context.Background()
	owner := testDB(t, "OWNER")
	adminRaw, e := os.ReadFile(os.Getenv("LMM_CHECKINS_TEST_ADMIN_URL_FILE"))
	if e != nil {
		t.Fatal(e)
	}
	admin, e := sql.Open("postgres", strings.TrimSpace(string(adminRaw)))
	if e != nil {
		t.Fatal(e)
	}
	defer admin.Close()
	var marker string
	if e = admin.QueryRow("SELECT current_setting('lmm.synthetic',true)").Scan(&marker); e != nil || marker != "checkins-extension-test" {
		t.Fatal("admin is not synthetic")
	}
	if _, e = owner.Exec(Schema + "\n" + SettingsSchema); e != nil {
		t.Fatal(e)
	}
	const role = "checkins_test_runtime"
	const operation = "legacy-history-first-operation"
	x := historyFixture()
	raw, hash := historyBytes(x)
	importRows := func(op string, b []byte, h string, recoverOnly, commit bool) (LegacyHistoryReceipt, error) {
		t.Helper()
		tx, e := owner.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if e != nil {
			t.Fatal(e)
		}
		defer tx.Rollback()
		r, e := ImportLegacyHistory(ctx, tx, role, op, b, h, recoverOnly)
		if e == nil && commit && !recoverOnly {
			e = tx.Commit()
		}
		return r, e
	}
	if _, e = importRows(operation, raw, hash, false, false); e != nil {
		t.Fatal("rollback import", e)
	}
	var exists bool
	if e = owner.QueryRow("SELECT to_regclass('lmm_checkins.legacy_history') IS NOT NULL").Scan(&exists); e != nil || exists {
		t.Fatal("rolled-back DDL became durable", e)
	}
	if _, e = importRows(operation, raw, hash, true, false); e != ErrConflict {
		t.Fatal("recovery manufactured a missing receipt", e)
	}
	original, e := importRows(operation, raw, hash, false, true)
	if e != nil || original.SourceRows != 4 || original.MappedRows != 3 || original.ArchivedRows != 1 || original.AddedRows != 4 {
		t.Fatal("import", original, e)
	}
	// The response can be lost after COMMIT. Recovery reads the original database
	// receipt, without another INSERT or an invented success result.
	recovered, e := importRows(operation, raw, hash, true, false)
	if e != nil || recovered != original {
		t.Fatal("committed original receipt was not recovered", recovered, e)
	}
	replayed, e := importRows(operation, raw, hash, false, true)
	if e != nil || replayed != original {
		t.Fatal("original operation replay", replayed, e)
	}
	if _, e = owner.Exec(`GRANT USAGE ON SCHEMA lmm_checkins TO checkins_test_runtime;
 GRANT SELECT ON lmm_checkins.contract,lmm_checkins.policy_revisions,lmm_checkins.active_policy,lmm_checkins.intentions,lmm_checkins.claim_commands TO checkins_test_runtime;
 GRANT INSERT ON lmm_checkins.intentions,lmm_checkins.claim_commands TO checkins_test_runtime;
 GRANT UPDATE(state,record) ON lmm_checkins.intentions TO checkins_test_runtime;
 GRANT UPDATE(result) ON lmm_checkins.claim_commands TO checkins_test_runtime`); e != nil {
		t.Fatal(e)
	}
	p := Policy{Enabled: true, MinQuota: "100", MaxQuota: "100", CalendarOffsetSeconds: Offset, CreditsPerUSD: 500000, SourceRevision: SourceRevision}
	if _, e = PublishPolicy(ctx, owner, "history-test-reward-policy", 0, p); e != nil {
		t.Fatal(e)
	}
	if _, e = admin.Exec("ALTER ROLE checkins_test_runtime LOGIN"); e != nil {
		t.Fatal(e)
	}
	runtime := testDB(t, "RUNTIME")
	l := &fakeLedger{a: Authority{UserID: 501, AccountID: 733, Timestamp: x.ObservedAt, Day: "2030-01-02", MaxDailyUnits: 100}, receipts: map[string]Receipt{}, challenges: map[string]string{}, authorizations: map[string]string{}}
	s, e := New(ctx, runtime, l)
	if e != nil {
		t.Fatal(e)
	}
	status, e := s.Status(ctx, "fresh-personal-session", "")
	if e != nil || !status.CheckedInToday || status.Pending || status.TotalCheckins != 2 || status.TotalQuotaAwarded != "125" || len(status.Records) != 1 || status.TodayRecord.Origin != "history" || status.TodayRecord.JournalID != nil || status.TodayRecord.PolicyRevision != nil {
		t.Fatal("historical status", status, e)
	}
	previous, e := s.Status(ctx, "fresh-personal-session", "2029-12")
	if e != nil || len(previous.Records) != 1 || previous.Records[0].CheckinDate != "2029-12-31" || !previous.CheckedInToday {
		t.Fatal("selected month changed today's state", previous, e)
	}
	claim, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: "historical-day-claim-0001"})
	if e != nil || !claim.AlreadyClaimed || claim.Record.Origin != "history" || claim.Record.JournalID != nil {
		t.Fatal("historical claim", claim, e)
	}
	if _, e = s.Challenge(ctx, "fresh-personal-session", "historical-day-challenge"); e != ErrConflict {
		t.Fatal("historical challenge", e)
	}
	if l.inspects == 0 || l.challengeCount != 0 || l.authorizes != 0 || l.posts != 0 {
		t.Fatal("historical award crossed financial RPC boundary", l.inspects, l.challengeCount, l.authorizes, l.posts)
	}
	var intentions, commands int
	if e = owner.QueryRow("SELECT (SELECT count(*) FROM lmm_checkins.intentions),(SELECT count(*) FROM lmm_checkins.claim_commands)").Scan(&intentions, &commands); e != nil || intentions != 0 || commands != 0 {
		t.Fatal("history was seeded into native settlement state", intentions, commands, e)
	}
	if _, e = runtime.Exec("UPDATE lmm_checkins.legacy_history SET amount_units=amount_units"); e == nil {
		t.Fatal("runtime can update historical awards")
	}
	if _, e = runtime.Exec("DELETE FROM lmm_checkins.legacy_history"); e == nil {
		t.Fatal("runtime can delete historical awards")
	}
	if _, e = runtime.Query("SELECT manifest FROM lmm_checkins.legacy_history_imports"); e == nil {
		t.Fatal("runtime can read private import material")
	}
	if _, e = owner.Exec("UPDATE lmm_checkins.legacy_history SET amount_units=amount_units"); e == nil {
		t.Fatal("owner can overwrite immutable history")
	}
	if _, e = owner.Exec("DELETE FROM lmm_checkins.legacy_history_imports"); e == nil {
		t.Fatal("owner can remove the original receipt")
	}
	// A source user without a committed mapping is archival, even if an unrelated
	// native user happens to have that same numeric ID.
	l.a.UserID, l.a.AccountID = 99, 1999
	orphan, e := s.Status(ctx, "fresh-personal-session", "")
	if e != nil || orphan.CheckedInToday || orphan.TotalCheckins != 0 {
		t.Fatal("orphan was assigned by numeric identity", orphan, e)
	}
	l.a.UserID, l.a.AccountID = 501, 734
	if _, e = s.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: "wrong-account-claim-0001"}); e != ErrConflict {
		t.Fatal("wrong personal account accepted", e)
	}
	l.a.AccountID = 733
	// Cross the actual UTC+08:00 day boundary. The next day uses ordinary Core
	// verification and a new native settlement; historical receipts stay intact.
	l.a.Timestamp = time.Date(2030, 1, 2, 16, 0, 0, 0, time.UTC).Unix()
	l.a.Day = day(l.a.Timestamp)
	if l.a.Day != "2030-01-03" {
		t.Fatal("calendar drift")
	}
	next, e := s.Claim(ctx, "fresh-personal-session", proof(t, s, "native-next-day-operation"))
	if e != nil || next.AlreadyClaimed || next.Record.Origin != "native" || next.Record.JournalID == nil || l.posts != 1 || l.authorizes != 1 || l.challengeCount != 1 {
		t.Fatal("next day failed normal authorization", next, e)
	}
	combined, e := s.Status(ctx, "fresh-personal-session", "")
	if e != nil || combined.TotalCheckins != 3 || combined.TotalQuotaAwarded != "225" || len(combined.Records) != 2 {
		t.Fatal("native/history totals", combined, e)
	}
	var nativeBefore string
	if e = owner.QueryRow("SELECT jsonb_agg(to_jsonb(i) ORDER BY account_id,day)::text FROM lmm_checkins.intentions i").Scan(&nativeBefore); e != nil {
		t.Fatal(e)
	}
	if _, e = importRows("forbidden-live-import-0001", raw, hash, false, true); e != ErrUnavailable {
		t.Fatal("live runtime was not fenced", e)
	}
	runtime.Close()
	if _, e = admin.Exec("ALTER ROLE checkins_test_runtime NOLOGIN"); e != nil {
		t.Fatal(e)
	}
	bad := historyFixture()
	bad.Mapping[0].TargetPersonalAccountID = 734
	m, _ := json.Marshal(bad.Mapping)
	bad.IdentityMappingSHA256 = digestHistory(m)
	b, h := historyBytes(bad)
	if _, e = importRows("wrong-map-import-operation", b, h, false, true); e != ErrConflict {
		t.Fatal("committed owner remapped", e)
	}
	bad = historyFixture()
	bad.ObservedAt = l.a.Timestamp
	bad.Rows = append(bad.Rows, LegacyHistoryRecord{ID: 14, UserID: 1, CheckinDate: l.a.Day, QuotaAwarded: 100, CreatedAt: l.a.Timestamp})
	bad.SourceRowCount++
	b, h = historyBytes(bad)
	if _, e = importRows("native-overlap-import-0001", b, h, false, true); e != ErrConflict {
		t.Fatal("native settled day overlapped", e)
	}
	if _, e = importRows(operation, raw, hash, true, false); e != nil {
		t.Fatal("native activity invalidated old receipt recovery", e)
	}
	var nativeAfter string
	if e = owner.QueryRow("SELECT jsonb_agg(to_jsonb(i) ORDER BY account_id,day)::text FROM lmm_checkins.intentions i").Scan(&nativeAfter); e != nil || nativeBefore != nativeAfter {
		t.Fatal("import/recovery changed a native record", e)
	}
	// A changed request under the same operation is a conflict, not a new import.
	bad = historyFixture()
	bad.SourceExportSHA256 = strings.Repeat("f", 64)
	b, h = historyBytes(bad)
	if _, e = importRows(operation, b, h, false, true); !errors.Is(e, ErrConflict) {
		t.Fatal("operation identity changed", e)
	}
}
