package checkins

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	_ "github.com/lib/pq"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func testDB(t *testing.T, label string) *sql.DB {
	t.Helper()
	path := os.Getenv("LMM_CHECKINS_TEST_" + label + "_URL_FILE")
	if path == "" {
		t.Skip("requires explicitly provisioned synthetic checkins database")
	}
	raw, e := os.ReadFile(path)
	if e != nil {
		t.Fatal(e)
	}
	db, e := sql.Open("postgres", strings.TrimSpace(string(raw)))
	if e != nil {
		t.Fatal(e)
	}
	db.SetMaxOpenConns(2)
	t.Cleanup(func() { db.Close() })
	var marker, name string
	if e = db.QueryRow("SELECT current_database(),current_setting('lmm.synthetic',true)").Scan(&name, &marker); e != nil || marker != "checkins-extension-test" || !strings.HasPrefix(name, "lmm_checkins_") {
		t.Fatalf("not synthetic checkins storage: %v", e)
	}
	if label == "OWNER" {
		db.SetMaxOpenConns(1)
		if _, e = db.Exec("SET ROLE checkins_test_owner"); e != nil {
			t.Fatal(e)
		}
	}
	return db
}

type fakeLedger struct {
	mu               sync.Mutex
	a                Authority
	receipts         map[string]Receipt
	challenges       map[string]string
	authorizations   map[string]string
	challengeCount   int
	authorizes       int
	expireBeforePost bool
	lost             bool
	posts            int
	inspects         int
}

func (l *fakeLedger) Inspect(_ context.Context, user string) (Authority, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.inspects++
	if user != "fresh-personal-session" {
		return Authority{}, ErrUnauthorized
	}
	return l.a, nil
}
func (l *fakeLedger) Challenge(_ context.Context, user, key string) (Challenge, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if user != "fresh-personal-session" {
		return Challenge{}, ErrUnauthorized
	}
	l.challengeCount++
	id := fmt.Sprintf("synthetic-challenge-%016d", l.challengeCount)
	l.challenges[id] = key
	return Challenge{ChallengeID: id, OperationKey: key, Day: l.a.Day, ExpiresAt: l.a.Timestamp + 120, TurnstileSiteKey: "synthetic-site", TurnstileAction: "daily_checkin", TurnstileCData: id}, nil
}
func (l *fakeLedger) Authorize(_ context.Context, user string, input ClaimRequest) (Authorization, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.authorizes++
	if user != "fresh-personal-session" {
		return Authorization{}, ErrUnauthorized
	}
	if l.challenges[input.ChallengeID] != input.OperationKey || input.TurnstileToken != "synthetic-verified-token" {
		return Authorization{}, ErrVerificationFailed
	}
	l.authorizations[input.ChallengeID] = input.OperationKey
	return Authorization{ID: input.ChallengeID, Day: l.a.Day, ExpiresAt: l.a.Timestamp + 120}, nil
}
func proof(t *testing.T, s *Service, key string) ClaimRequest {
	t.Helper()
	challenge, e := s.Challenge(context.Background(), "fresh-personal-session", key)
	if e != nil {
		t.Fatal(e)
	}
	return ClaimRequest{OperationKey: challenge.OperationKey, ChallengeID: challenge.ChallengeID, TurnstileToken: "synthetic-verified-token"}
}
func (l *fakeLedger) Post(_ context.Context, user string, input Posting) (Receipt, error) {
	date, amount := input.Day, input.Amount
	l.mu.Lock()
	defer l.mu.Unlock()
	if user != "fresh-personal-session" {
		return Receipt{}, ErrUnauthorized
	}
	if r, ok := l.receipts[date]; ok {
		r.Replayed = true
		return r, nil
	}
	if l.a.Blocked || amount > l.a.MaxDailyUnits {
		return Receipt{}, ErrForbidden
	}
	if date != l.a.Day {
		return Receipt{}, ErrVerificationRequired
	}
	if l.expireBeforePost {
		delete(l.authorizations, input.AuthorizationID)
		l.expireBeforePost = false
	}
	if input.AuthorizationID == "" || l.authorizations[input.AuthorizationID] != input.OperationKey {
		return Receipt{}, ErrVerificationRequired
	}
	delete(l.authorizations, input.AuthorizationID)
	l.posts++
	r := Receipt{UserID: l.a.UserID, AccountID: l.a.AccountID, Day: date, AmountUnits: amount, Timestamp: l.a.Timestamp}
	if amount > 0 {
		id := int64(l.posts)
		r.JournalID = &id
	}
	l.receipts[date] = r
	if l.lost {
		l.lost = false
		return Receipt{}, ErrUnavailable
	}
	return r, nil
}

var accountCounter atomic.Int64

func newLedger() *fakeLedger {
	id := time.Now().UnixNano() + accountCounter.Add(1)
	at := time.Now().Unix()
	return &fakeLedger{a: Authority{UserID: id, AccountID: id + 1, Timestamp: at, Day: day(at), MaxDailyUnits: 100}, receipts: map[string]Receipt{}, challenges: map[string]string{}, authorizations: map[string]string{}}
}
func TestRealPostgresDailyIntentionAndRecovery(t *testing.T) {
	ctx := context.Background()
	owner := testDB(t, "OWNER")
	runtime := testDB(t, "RUNTIME")
	var expected int64
	if e := owner.QueryRow("SELECT coalesce(max(revision),0) FROM lmm_checkins.policy_revisions").Scan(&expected); e != nil {
		t.Fatal(e)
	}
	p := Policy{Enabled: true, MinQuota: "100", MaxQuota: "100", CalendarOffsetSeconds: Offset, CreditsPerUSD: 500000, SourceRevision: SourceRevision}
	publish := func(p Policy) {
		t.Helper()
		r, e := PublishPolicy(ctx, owner, "test-policy-"+time.Now().Format("20060102150405.000000000"), expected, p)
		if e != nil {
			t.Fatal(e)
		}
		expected = r
	}
	publish(p)
	if _, e := runtime.Exec("INSERT INTO lmm_checkins.policy_revisions VALUES(999999,'{}','unauthorized-key-000',0)"); e == nil {
		t.Fatal("runtime can publish reward policy")
	}
	if _, e := runtime.Exec("UPDATE lmm_checkins.active_policy SET revision=revision"); e == nil {
		t.Fatal("runtime can enable reward policy")
	}
	l := newLedger()
	s, e := New(ctx, runtime, l)
	if e != nil {
		t.Fatal(e)
	}
	key := "daily-first-operation-0001"
	// A request without proof must not freeze a reward or invoke funding.
	if _, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: key}); e != ErrVerificationRequired {
		t.Fatalf("missing proof: %v", e)
	}
	badProof := proof(t, s, key)
	badProof.TurnstileToken = "forged-token"
	if _, e := s.Claim(ctx, "fresh-personal-session", badProof); e != ErrVerificationFailed {
		t.Fatalf("forged proof: %v", e)
	}
	var deniedCount int
	if e := runtime.QueryRow("SELECT count(*) FROM lmm_checkins.intentions WHERE account_id=$1", l.a.AccountID).Scan(&deniedCount); e != nil || deniedCount != 0 || l.posts != 0 {
		t.Fatalf("unverified claim created intention or funds: %d %d %v", deniedCount, l.posts, e)
	}
	first, e := s.Claim(ctx, "fresh-personal-session", proof(t, s, key))
	if e != nil {
		t.Fatal(e)
	}
	if first.AlreadyClaimed || first.Record.QuotaAwarded != "100" || first.Record.JournalID == nil {
		t.Fatalf("incorrect first receipt: %+v", first)
	}
	p.Enabled = false
	publish(p)
	verifiedBeforeReplay := l.authorizes
	same, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: key})
	if e != nil {
		t.Fatal(e)
	}
	a, _ := json.Marshal(first)
	b, _ := json.Marshal(same)
	if string(a) != string(b) {
		t.Fatal("same operation changed after policy publication")
	}
	other, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: "daily-second-operation-0002"})
	if e != nil || !other.AlreadyClaimed || other.Record.QuotaAwarded != "100" {
		t.Fatalf("daily replay: %+v %v", other, e)
	}
	if l.posts != 1 || l.authorizes != verifiedBeforeReplay {
		t.Fatalf("replay posted %d times or reverified: %d", l.posts, l.authorizes)
	}
	status, e := s.Status(ctx, "fresh-personal-session", "")
	if e != nil || status.Enabled || status.TotalCheckins != 1 || status.TotalQuotaAwarded != "100" || len(status.Records) != 1 || !status.CheckedInToday {
		t.Fatalf("status %+v %v", status, e)
	}
	if _, e = runtime.Exec("UPDATE lmm_checkins.intentions SET amount_units=amount_units+1 WHERE account_id=$1", l.a.AccountID); e == nil {
		t.Fatal("runtime changed frozen monetary intention")
	}
	if _, e = runtime.Exec("UPDATE lmm_checkins.intentions SET record=record WHERE account_id=$1", l.a.AccountID); e == nil {
		t.Fatal("settled record mutable")
	}
	if _, e = runtime.Exec("DELETE FROM lmm_checkins.intentions WHERE account_id=$1", l.a.AccountID); e == nil {
		t.Fatal("runtime deleted durable receipt")
	}
	p.Enabled = true
	publish(p)
	unknown := newLedger()
	unknown.lost = true
	recovering, _ := New(ctx, runtime, unknown)
	if _, e = recovering.Claim(ctx, "fresh-personal-session", proof(t, recovering, "unknown-result-operation-001")); !errors.Is(e, ErrUnavailable) {
		t.Fatalf("unknown result reported as %v", e)
	}
	pending, e := recovering.Status(ctx, "fresh-personal-session", "")
	if e != nil || !pending.Pending || pending.CheckedInToday || pending.TotalCheckins != 0 {
		t.Fatalf("pending status %+v %v", pending, e)
	}
	oldDate := unknown.a.Day
	unknown.a.Timestamp += 86400
	unknown.a.Day = day(unknown.a.Timestamp)
	unknown.a.MaxDailyUnits = 0
	unknown.a.Blocked = true
	p.Enabled = false
	publish(p)
	recovered, e := recovering.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: "unknown-result-operation-001"})
	if e != nil || recovered.Record.CheckinDate != oldDate || recovered.Record.QuotaAwarded != "100" || unknown.posts != 1 || unknown.authorizes != 1 {
		t.Fatalf("lost receipt retry %+v %v", recovered, e)
	}
	p.Enabled = true
	publish(p)
	parallel := newLedger()
	concurrent, _ := New(ctx, runtime, parallel)
	var wg sync.WaitGroup
	errs := make(chan error, 8)
	proofs := make([]ClaimRequest, 8)
	for n := range proofs {
		proofs[n] = proof(t, concurrent, "parallel-operation-0001")
	}
	for n := 0; n < 8; n++ {
		wg.Add(1)
		go func(n int) {
			defer wg.Done()
			var e error
			for tries := 0; tries < 20; tries++ {
				_, e = concurrent.Claim(ctx, "fresh-personal-session", proofs[n])
				if e == nil {
					break
				}
				if !errors.Is(e, ErrUnavailable) {
					break
				}
			}
			errs <- e
		}(n)
	}
	wg.Wait()
	close(errs)
	for e := range errs {
		if e != nil {
			t.Fatal(e)
		}
	}
	if parallel.posts != 1 {
		t.Fatalf("concurrent posting count %d", parallel.posts)
	}
	var count int
	if e = runtime.QueryRow("SELECT count(*) FROM lmm_checkins.intentions WHERE account_id=$1", parallel.a.AccountID).Scan(&count); e != nil || count != 1 {
		t.Fatalf("daily intention count %d %v", count, e)
	}
	expired := newLedger()
	expired.expireBeforePost = true
	expiredService, _ := New(ctx, runtime, expired)
	original := proof(t, expiredService, "expired-original-operation-001")
	if _, e := expiredService.Claim(ctx, "fresh-personal-session", original); e != ErrVerificationRequired {
		t.Fatalf("expired before funding: %v", e)
	}
	before, e := expiredService.Status(ctx, "fresh-personal-session", "")
	if e != nil || !before.Pending || expired.posts != 0 {
		t.Fatalf("expired pending %+v %v", before, e)
	}
	if _, e := expiredService.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: original.OperationKey}); e != ErrVerificationRequired || expired.posts != 0 {
		t.Fatalf("pending bypassed proof: %v posts=%d", e, expired.posts)
	}
	p.MinQuota = "7"
	p.MaxQuota = "7"
	publish(p)
	refreshed := proof(t, expiredService, "browser-reload-operation-002")
	if refreshed.OperationKey != original.OperationKey {
		t.Fatal("challenge refresh replaced original funding key")
	}
	finished, e := expiredService.Claim(ctx, "fresh-personal-session", refreshed)
	if e != nil || finished.Record.QuotaAwarded != "100" || expired.posts != 1 {
		t.Fatalf("renewed proof changed frozen reward %+v %v", finished, e)
	}
	previousDay := newLedger()
	previousDay.expireBeforePost = true
	previousDayService, _ := New(ctx, runtime, previousDay)
	previousInput := proof(t, previousDayService, "previous-day-operation-001")
	if _, e := previousDayService.Claim(ctx, "fresh-personal-session", previousInput); e != ErrVerificationRequired {
		t.Fatalf("previous-day pending setup: %v", e)
	}
	previousDate := previousDay.a.Day
	previousDay.a.Timestamp += 86400
	previousDay.a.Day = day(previousDay.a.Timestamp)
	if _, e := previousDayService.Challenge(ctx, "fresh-personal-session", previousInput.OperationKey); e != ErrConflict || previousDay.challengeCount != 1 {
		t.Fatalf("old command received a new-day proof: %v", e)
	}
	if _, e := previousDayService.Claim(ctx, "fresh-personal-session", ClaimRequest{OperationKey: previousInput.OperationKey}); e != ErrVerificationRequired || previousDay.posts != 0 {
		t.Fatalf("old unposted intention crossed day: %v", e)
	}
	currentInput := proof(t, previousDayService, "current-day-operation-002")
	current, e := previousDayService.Claim(ctx, "fresh-personal-session", currentInput)
	if e != nil || current.Record.CheckinDate != previousDay.a.Day || current.Record.QuotaAwarded != "7" || previousDay.posts != 1 {
		t.Fatalf("new-day operation %+v %v", current, e)
	}
	var preservedDate string
	if e := runtime.QueryRow("SELECT day::text FROM lmm_checkins.claim_commands WHERE user_id=$1 AND operation_key=$2", previousDay.a.UserID, previousInput.OperationKey).Scan(&preservedDate); e != nil || preservedDate != previousDate {
		t.Fatalf("new challenge rewrote previous-day command: %s %v", preservedDate, e)
	}
	p.MinQuota = "0"
	p.MaxQuota = "0"
	publish(p)
	zero := newLedger()
	zero.a.MaxDailyUnits = 0
	zeroService, _ := New(ctx, runtime, zero)
	z, e := zeroService.Claim(ctx, "fresh-personal-session", proof(t, zeroService, "zero-reward-operation-001"))
	if e != nil || z.Record.JournalID != nil || z.Record.QuotaAwarded != "0" {
		t.Fatalf("zero reward %+v %v", z, e)
	}
}
func TestRewardRangeUsesConfiguredBaseForAllUsers(t *testing.T) {
	var p Policy
	if err := json.Unmarshal([]byte(`{"min_quota":"1","max_quota":"3","level_multipliers":[0.5,0.65,0.8,0.9,1]}`), &p); err != nil {
		t.Fatal(err)
	}
	r, err := p.rewardRange()
	if err != nil || r.MinQuota != "1" || r.MaxQuota != "3" || r.RewardMultiplier != 1 {
		t.Fatalf("base rewards changed by legacy multipliers: %+v %v", r, err)
	}
	p.MinQuota, p.MaxQuota = "9007199254740993", "9007199254740995"
	r, err = p.rewardRange()
	if err != nil || r.MinQuota != p.MinQuota || r.MaxQuota != p.MaxQuota {
		t.Fatalf("integer reward range lost precision: %+v %v", r, err)
	}
}

func TestHistoricalRecordStorageDoesNotExposeTrustLevel(t *testing.T) {
	stored := json.RawMessage(`{"checkin_date":"2026-11-15","quota_awarded":"7","created_at":1794715200,"journal_id":1,"origin":"native","policy_revision":2,"trust_level":4}`)
	var record DailyRecord
	if err := json.Unmarshal(stored, &record); err != nil {
		t.Fatal(err)
	}
	result := ClaimResult{AlreadyClaimed: true, Record: record}
	public, err := json.Marshal(result)
	if err != nil || strings.Contains(string(public), "trust_level") || record.QuotaAwarded != "7" {
		t.Fatalf("historical public projection: %s %v", public, err)
	}
	persisted, err := storedClaimResult(result, stored)
	var decoded struct {
		AlreadyClaimed bool `json:"already_claimed"`
		Record struct { Level uint32 `json:"trust_level"` } `json:"record"`
	}
	if err != nil || json.Unmarshal(persisted, &decoded) != nil || !decoded.AlreadyClaimed || decoded.Record.Level != 4 {
		t.Fatalf("historical durable record changed: %s %v", persisted, err)
	}
}
