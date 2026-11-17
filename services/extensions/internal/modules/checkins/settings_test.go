// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package checkins

import (
	"context"
	"database/sql"
	"os"
	"strings"
	"testing"
)

func TestSettingsAmountsAndUniformRewardsRemainExact(t *testing.T) {
	input := SettingsInput{Revision: 1, OperationKey: "settings-exact-operation", Enabled: true, MinQuota: "9007199254740993", MaxQuota: "9223372036854775807"}
	if input.validate() != nil {
		t.Fatal("exact i64 amounts rejected")
	}
	p := Policy{Enabled: true, MinQuota: input.MinQuota, MaxQuota: input.MaxQuota, CalendarOffsetSeconds: Offset, CreditsPerUSD: 500000, SourceRevision: SourceRevision}
	r, e := p.rewardRange()
	if e != nil || r.RewardMultiplier != 1 || r.MinQuota != input.MinQuota || r.MaxQuota != input.MaxQuota {
		t.Fatal("uniform reward lost precision", r, e)
	}
	for _, invalid := range []string{"-1", "01", "1.5", "1e3", "9223372036854775808"} {
		bad := input
		bad.MinQuota = invalid
		if bad.validate() == nil {
			t.Fatal("invalid amount accepted", invalid)
		}
	}
	input.MaxQuota = "1"
	if input.validate() == nil {
		t.Fatal("inverted range accepted")
	}
}

func TestSettingsPostgresUpgradePersistenceAndConflicts(t *testing.T) {
	owner := testDB(t, "OWNER")
	ctx := context.Background()
	if _, e := owner.Exec(Schema); e != nil {
		t.Fatal(e)
	}
	const role = "checkins_test_runtime"
	const operation = "checkin-settings-upgrade"
	for _, commit := range []bool{false, true} {
		tx, e := owner.BeginTx(ctx, nil)
		if e != nil {
			t.Fatal(e)
		}
		if e = UpgradeSettings(ctx, tx, role, operation, false); e != nil {
			t.Fatal("upgrade", e)
		}
		if commit {
			e = tx.Commit()
		} else {
			e = tx.Rollback()
		}
		if e != nil {
			t.Fatal(e)
		}
	}
	tx, e := owner.BeginTx(ctx, nil)
	if e != nil {
		t.Fatal(e)
	}
	if e = UpgradeSettings(ctx, tx, role, operation, true); e != nil {
		t.Fatal("recovery", e)
	}
	tx.Rollback()
	tx, e = owner.BeginTx(ctx, nil)
	if e != nil {
		t.Fatal(e)
	}
	if e = UpgradeSettings(ctx, tx, role, "different-upgrade-operation", true); e != ErrConflict {
		t.Fatal("different recovery", e)
	}
	tx.Rollback()
	raw, e := os.ReadFile(os.Getenv("LMM_CHECKINS_TEST_ADMIN_URL_FILE"))
	if e != nil {
		t.Fatal(e)
	}
	admin, e := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if e != nil {
		t.Fatal(e)
	}
	if _, e = admin.Exec("ALTER ROLE checkins_test_runtime LOGIN"); e != nil {
		t.Fatal(e)
	}
	admin.Close()
	if _, e = owner.Exec(`GRANT USAGE ON SCHEMA lmm_checkins TO checkins_test_runtime;
 GRANT SELECT ON lmm_checkins.contract,lmm_checkins.policy_revisions,lmm_checkins.active_policy,lmm_checkins.intentions,lmm_checkins.claim_commands TO checkins_test_runtime;
 GRANT INSERT ON lmm_checkins.intentions,lmm_checkins.claim_commands TO checkins_test_runtime;
 GRANT UPDATE(state,record) ON lmm_checkins.intentions TO checkins_test_runtime;
 GRANT UPDATE(result) ON lmm_checkins.claim_commands TO checkins_test_runtime;`); e != nil {
		t.Fatal(e)
	}
	runtime := testDB(t, "RUNTIME")
	a := Authority{UserID: 501, AccountID: 501, Timestamp: 1794956400, Day: day(1794956400), MaxDailyUnits: 50000}
	ledger := &fakeLedger{a: a}
	service, e := New(ctx, runtime, ledger)
	if e != nil {
		t.Fatal(e)
	}
	initial, e := service.Settings(ctx, "fresh-personal-session")
	if e != nil || !initial.Writable || initial.Revision != 0 || initial.Enabled || !initial.FundingAvailable {
		t.Fatal(initial, e)
	}
	input := SettingsInput{OperationKey: "settings-first-operation", Enabled: true, MinQuota: "1000", MaxQuota: "2000"}
	saved, e := service.SaveSettings(ctx, "fresh-personal-session", 501, input)
	if e != nil || saved.Revision != 1 || !saved.Enabled {
		t.Fatal(saved, e)
	}
	var hasLegacyMultipliers bool
	if e = owner.QueryRow("SELECT policy ? 'level_multipliers' FROM lmm_checkins.policy_revisions WHERE revision=$1", saved.Revision).Scan(&hasLegacyMultipliers); e != nil || hasLegacyMultipliers {
		t.Fatal("new settings retained retired reward levels", hasLegacyMultipliers, e)
	}
	replay, e := service.SaveSettings(ctx, "fresh-personal-session", 501, input)
	if e != nil || replay.Revision != saved.Revision {
		t.Fatal("replay", replay, e)
	}
	stale := input
	stale.OperationKey = "settings-stale-operation"
	if _, e = service.SaveSettings(ctx, "fresh-personal-session", 501, stale); e != ErrConflict {
		t.Fatal("stale write", e)
	}
	mismatch := input
	mismatch.MaxQuota = "3000"
	if _, e = service.SaveSettings(ctx, "fresh-personal-session", 501, mismatch); e != ErrConflict {
		t.Fatal("operation mismatch", e)
	}
	high := input
	high.Revision = 1
	high.OperationKey = "settings-over-cap-operation"
	high.MaxQuota = "50001"
	if _, e = service.SaveSettings(ctx, "fresh-personal-session", 501, high); e != ErrInvalid {
		t.Fatal("Core ceiling ignored", e)
	}
	restarted, e := New(ctx, runtime, ledger)
	if e != nil {
		t.Fatal(e)
	}
	after, e := restarted.Settings(ctx, "fresh-personal-session")
	if e != nil || after.Revision != 1 || after.MinQuota != "1000" || after.MaxQuota != "2000" {
		t.Fatal("restart", after, e)
	}
	if _, e = runtime.Exec("UPDATE lmm_checkins.active_policy SET revision=2"); e == nil {
		t.Fatal("direct policy write allowed")
	}
	if _, e = runtime.Exec("DELETE FROM lmm_checkins.policy_revisions"); e == nil {
		t.Fatal("history deletion allowed")
	}
	unavailable, e := restarted.Settings(ctx, "offline-funding")
	if e != nil || unavailable.FundingAvailable || unavailable.Revision != 1 || !unavailable.Writable {
		t.Fatal("funding outage hid persisted settings", unavailable, e)
	}
	// A subsequent edit drops old multiplier metadata without rewriting history.
	if _, e = owner.Exec(`INSERT INTO lmm_checkins.policy_revisions(revision,policy,operation_key,created_at)
 SELECT 2,policy || '{"level_multipliers":[0,1,2,3,4]}'::jsonb,'settings-legacy-policy',created_at
	 FROM lmm_checkins.policy_revisions WHERE revision=1;
	 UPDATE lmm_checkins.active_policy SET revision=2 WHERE singleton;`); e != nil {
		t.Fatal(e)
	}
	next := input
	next.Revision = 2
	next.OperationKey = "settings-flat-reward-policy"
	flat, e := restarted.SaveSettings(ctx, "fresh-personal-session", 501, next)
	if e != nil || flat.Revision != 3 || flat.MinQuota != input.MinQuota || flat.MaxQuota != input.MaxQuota {
		t.Fatal("legacy settings edit", flat, e)
	}
	var historicalHasMultipliers bool
	if e = owner.QueryRow(`SELECT
 (SELECT policy ? 'level_multipliers' FROM lmm_checkins.policy_revisions WHERE revision=2),
 (SELECT policy ? 'level_multipliers' FROM lmm_checkins.policy_revisions WHERE revision=3)`).Scan(&historicalHasMultipliers, &hasLegacyMultipliers); e != nil || !historicalHasMultipliers || hasLegacyMultipliers {
		t.Fatal("legacy history or flat current settings changed incorrectly", historicalHasMultipliers, hasLegacyMultipliers, e)
	}
}
