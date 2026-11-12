package checkins

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
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
	mu       sync.Mutex
	a        Authority
	receipts map[string]Receipt
	lost     bool
	posts    int
	inspects int
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
func (l *fakeLedger) Post(_ context.Context, user, date string, amount int64, level uint32) (Receipt, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if user != "fresh-personal-session" {
		return Receipt{}, ErrUnauthorized
	}
	if r, ok := l.receipts[date]; ok {
		r.Replayed = true
		return r, nil
	}
	if l.a.Blocked || amount > l.a.MaxDailyUnits || level != l.a.TrustLevel || date != l.a.Day {
		return Receipt{}, ErrForbidden
	}
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
	return &fakeLedger{a: Authority{UserID: id, AccountID: id + 1, Timestamp: at, Day: day(at), TrustLevel: 4, MaxDailyUnits: 100}, receipts: map[string]Receipt{}}
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
	first, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{key})
	if e != nil {
		t.Fatal(e)
	}
	if first.AlreadyClaimed || first.Record.QuotaAwarded != "100" || first.Record.JournalID == nil {
		t.Fatalf("incorrect first receipt: %+v", first)
	}
	p.Enabled = false
	publish(p)
	same, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{key})
	if e != nil {
		t.Fatal(e)
	}
	a, _ := json.Marshal(first)
	b, _ := json.Marshal(same)
	if string(a) != string(b) {
		t.Fatal("same operation changed after policy publication")
	}
	other, e := s.Claim(ctx, "fresh-personal-session", ClaimRequest{"daily-second-operation-0002"})
	if e != nil || !other.AlreadyClaimed || other.Record.QuotaAwarded != "100" {
		t.Fatalf("daily replay: %+v %v", other, e)
	}
	if l.posts != 1 {
		t.Fatalf("posted %d times", l.posts)
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
	if _, e = recovering.Claim(ctx, "fresh-personal-session", ClaimRequest{"unknown-result-operation-001"}); !errors.Is(e, ErrUnavailable) {
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
	unknown.a.TrustLevel = 0
	unknown.a.Blocked = true
	p.Enabled = false
	publish(p)
	recovered, e := recovering.Claim(ctx, "fresh-personal-session", ClaimRequest{"unknown-result-operation-001"})
	if e != nil || recovered.Record.CheckinDate != oldDate || recovered.Record.QuotaAwarded != "100" || unknown.posts != 1 {
		t.Fatalf("lost receipt retry %+v %v", recovered, e)
	}
	p.Enabled = true
	publish(p)
	parallel := newLedger()
	concurrent, _ := New(ctx, runtime, parallel)
	var wg sync.WaitGroup
	errs := make(chan error, 8)
	for n := 0; n < 8; n++ {
		wg.Add(1)
		go func(n int) {
			defer wg.Done()
			var e error
			for tries := 0; tries < 20; tries++ {
				_, e = concurrent.Claim(ctx, "fresh-personal-session", ClaimRequest{"parallel-operation-000" + string(rune('a'+n))})
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
	p.MinQuota = "0"
	p.MaxQuota = "0"
	publish(p)
	zero := newLedger()
	zero.a.MaxDailyUnits = 0
	zeroService, _ := New(ctx, runtime, zero)
	z, e := zeroService.Claim(ctx, "fresh-personal-session", ClaimRequest{"zero-reward-operation-001"})
	if e != nil || z.Record.JournalID != nil || z.Record.QuotaAwarded != "0" {
		t.Fatalf("zero reward %+v %v", z, e)
	}
}
func TestRewardRangeMatchesOriginalGoRounding(t *testing.T) {
	p := Policy{MinQuota: "1", MaxQuota: "3", SourceRevision: SourceRevision, CalendarOffsetSeconds: Offset, CreditsPerUSD: 500000}
	r, e := p.rewardRange(0)
	if e != nil || r.MinQuota != "1" || r.MaxQuota != "2" {
		t.Fatalf("half away rounding %+v %v", r, e)
	}
	p.LevelMultipliers = []float64{-1}
	r, e = p.rewardRange(0)
	if e != nil || r.RewardMultiplier != .5 {
		t.Fatal("invalid tier multiplier lost original default")
	}
}
