// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"encoding/json"
	"encoding/xml"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func profileLiveInt(n int64) *int64 { return &n }

func TestProfileShareLiveLayouts(t *testing.T) {
	for _, lang := range []string{"en", "zh", "zh-TW", "fr", "ja", "ru", "vi"} {
		for _, layout := range []string{"profile", "badge", "models"} {
			q := url.Values{"layout": {layout}, "lang": {lang}, "title": {"<script> & \"test\""}}
			var o profileShareSVGOptions
			var err error
			if layout == "models" {
				o, _, err = parseProfileShareModelsSVGOptions(q)
			} else {
				o, err = parseProfileShareSVGOptions(q)
			}
			if err != nil {
				t.Fatal(err)
			}
			var svg string
			switch layout {
			case "profile":
				svg = renderProfileShareProfileSVG(o, &profileShareOwner{DisplayName: "Owner", Username: "owner", Role: 10}, []profileShareDay{{Day: 86400, Tokens: 23}}, 0)
			case "models":
				svg, err = renderProfileShareModelsSVG(o, profileShareModelUsage{Tokens: 23, Requests: 2, ModelCount: 1, Models: []profileShareModelRow{{ModelName: "<script>", Tokens: 23, Requests: 2}}}, 1, 2)
			default:
				svg = renderProfileShareSVG(o, profileShareUsage{Tokens: 23, Requests: 2})
			}
			if err != nil || strings.Contains(svg, "<script>") || !strings.Contains(svg, "&lt;script&gt;") {
				t.Fatalf("unsafe %s %s: %v", layout, lang, err)
			}
			d := xml.NewDecoder(strings.NewReader(svg))
			for {
				_, e := d.Token()
				if e == io.EOF {
					break
				}
				if e != nil {
					t.Fatalf("invalid XML %s %s: %v", layout, lang, e)
				}
			}
		}
	}
	for _, q := range []url.Values{{"layout": {"badge"}, "bg": {"red"}}, {"layout": {"profile"}, "period": {"30d"}}, {"layout": {"models"}, "period": {"all"}}, {"layout": {"models"}, "top": {"13"}}, {"layout": {"badge"}, "token": {"secret"}}} {
		var err error
		if q.Get("layout") == "models" {
			_, _, err = parseProfileShareModelsSVGOptions(q)
		} else {
			_, err = parseProfileShareSVGOptions(q)
		}
		if err == nil {
			t.Fatalf("invalid options accepted: %v", q)
		}
	}
}

func TestProfileShareLiveModelMoney(t *testing.T) {
	usage := profileShareModelUsage{CreditsPerUSD: 500000, CNYPerUSD: "7.125", PricingAvailable: true}
	if got, _ := profileShareModelQuota(500000, "USD", usage); got != "1.0000 USD" {
		t.Fatal(got)
	}
	if got, _ := profileShareModelQuota(500000, "CNY", usage); got != "7.1250 CNY" {
		t.Fatal(got)
	}
	if got, _ := profileShareModelQuota(1, "USD", usage); got != "0.000002 USD" {
		t.Fatal(got)
	}
	for _, sample := range []struct {
		raw  int64
		want string
	}{{0, "0 Credit"}, {500000, "500000 Credit"}, {9007199254740991, "9007199254740991 Credit"}} {
		if got, _ := profileShareModelQuota(sample.raw, "CREDIT", usage); got != sample.want {
			t.Fatalf("CREDIT changed integer %d: %s", sample.raw, got)
		}
	}
	usage.CreditsPerUSD = 1000000
	if got, _ := profileShareModelQuota(500000, "CREDIT", usage); got != "—" {
		t.Fatal("different denominator treated as public Credit", got)
	}
	usage.CreditsPerUSD = 500000
	usage.PricingAvailable = false
	if got, _ := profileShareModelQuota(0, "USD", usage); got != "—" {
		t.Fatalf("missing basis became zero: %s", got)
	}
	if got, _ := profileShareModelQuota(500000, "CREDIT", usage); got != "—" {
		t.Fatal("unknown pricing treated as known Credit", got)
	}
	usage.PricingAvailable = true
	usage.CNYPerUSD = "NaN"
	if got, _ := profileShareModelQuota(1, "CNY", usage); got != "—" {
		t.Fatal(got)
	}
	if got, _ := profileShareModelQuota(1, "CREDIT", usage); got != "1 Credit" {
		t.Fatal("CREDIT incorrectly depends on FX", got)
	}
}

type profileLiveRoundTrip func(*http.Request) (*http.Response, error)

func (f profileLiveRoundTrip) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
func profileLiveCore(t *testing.T, handler http.HandlerFunc) *CoreRead {
	t.Helper()
	srv := httptest.NewServer(handler)
	t.Cleanup(srv.Close)
	dest, _ := url.Parse(srv.URL)
	return &CoreRead{token: "synthetic-profile-share-purpose-token", client: &http.Client{Transport: profileLiveRoundTrip(func(r *http.Request) (*http.Response, error) {
		if r.Header.Get("Authorization") != "Bearer synthetic-profile-share-purpose-token" || r.Header.Get("X-Lmm-Protocol") != "1" {
			t.Error("missing private Core authentication")
		}
		clone := r.Clone(r.Context())
		u := *r.URL
		clone.URL = &u
		clone.URL.Scheme, clone.URL.Host = dest.Scheme, dest.Host
		return http.DefaultTransport.RoundTrip(clone)
	})}}
}

func TestProfileShareLiveUsesScopedCoreAndPublicWhitelist(t *testing.T) {
	now := time.Date(2026, 11, 10, 12, 0, 0, 0, time.UTC)
	var calls atomic.Int32
	core := profileLiveCore(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path != "/internal/community/v1/usage/user" {
			t.Errorf("unexpected Core path %s", r.URL.Path)
		}
		var in map[string]int64
		if e := json.NewDecoder(r.Body).Decode(&in); e != nil {
			t.Error(e)
		}
		if in["user_id"] != 52 || in["start"] != now.Add(-30*24*time.Hour).Unix() || in["end"] != now.Unix() || in["model_limit"] != 0 || in["bucket_seconds"] != 0 {
			t.Errorf("unscoped read %v", in)
		}
		_ = json.NewEncoder(w).Encode(CoreUserUsage{UserID: 52, Active: true, Name: "private display", CreditsPerUSD: 500000, Totals: CoreUsageTotals{Tokens: 1234, Requests: 7, Credits: profileLiveInt(123)}, ModelCount: 1})
	})
	s := &Service{core: core, origin: "https://example.test"}
	sh := &profileShare{UserID: 52, AggregateUsageEnabled: true, LinkedProfiles: []LinkedProfile{{Provider: "chatgpt", URL: "https://chatgpt.com/u/example"}}}
	rows := s.resolveProfileShareSources(context.Background(), sh, "30d", now)
	if rows[0].Status != "live" || *rows[0].Tokens != 1234 || *rows[0].Requests != 7 || rows[1].Status != "login_required" || calls.Load() != 1 {
		t.Fatalf("sources: %#v", rows)
	}
	raw, _ := json.Marshal(rows)
	for _, secret := range []string{"private display", "credits", "model_name", "user_id", "balance", "metadata"} {
		if strings.Contains(string(raw), secret) {
			t.Fatalf("private field escaped: %s", secret)
		}
	}
	sh.AggregateUsageEnabled = false
	rows = s.resolveProfileShareSources(context.Background(), sh, "30d", now)
	if calls.Load() != 1 || rows[0].Status != "disabled" || rows[0].Tokens != nil {
		t.Fatal("disabled share fetched/published usage")
	}
}

// This SQL driver exercises the actual HTTP handler, not PostgreSQL persistence.
// Real Core/Go/PG and browser acceptance remain a separate root-owned check.
type profileLiveStore struct {
	sync.Mutex
	share *profileShare
}

func (s *profileLiveStore) Connect(context.Context) (driver.Conn, error) {
	return &profileLiveConn{s}, nil
}
func (s *profileLiveStore) Driver() driver.Driver { return profileLiveDriver{} }

type profileLiveDriver struct{}

func (profileLiveDriver) Open(string) (driver.Conn, error) { return nil, errors.New("connector only") }

type profileLiveConn struct{ s *profileLiveStore }

func (*profileLiveConn) Prepare(string) (driver.Stmt, error) {
	return nil, errors.New("unexpected prepare")
}
func (*profileLiveConn) Close() error              { return nil }
func (*profileLiveConn) Begin() (driver.Tx, error) { return nil, errors.New("read only") }
func (c *profileLiveConn) QueryContext(_ context.Context, q string, args []driver.NamedValue) (driver.Rows, error) {
	c.s.Lock()
	defer c.s.Unlock()
	rows := &profileLiveRows{}
	if !strings.Contains(q, "FROM lmm_community.profile_shares WHERE ") {
		return nil, errors.New("unexpected SQL")
	}
	sh := c.s.share
	if sh == nil {
		return rows, nil
	}
	if len(args) != 1 {
		return nil, errors.New("unscoped SQL")
	}
	if strings.Contains(q, "token=$1") && args[0].Value != sh.Token {
		return rows, nil
	}
	if strings.Contains(q, "user_id=$1") && args[0].Value != sh.UserID {
		return rows, nil
	}
	linked, _ := json.Marshal(sh.LinkedProfiles)
	rows.values = []driver.Value{sh.UserID, sh.Token, sh.CreatedAt, sh.ModelUsageEnabled, sh.AggregateUsageEnabled, linked}
	return rows, nil
}

type profileLiveRows struct{ values []driver.Value }

func (*profileLiveRows) Columns() []string {
	return []string{"user_id", "token", "created_at", "model_usage_enabled", "aggregate_usage_enabled", "linked_profiles"}
}
func (*profileLiveRows) Close() error { return nil }
func (r *profileLiveRows) Next(dest []driver.Value) error {
	if r.values == nil {
		return io.EOF
	}
	copy(dest, r.values)
	r.values = nil
	return nil
}

func TestProfileShareLiveHTTPOriginalConsentAndRevocation(t *testing.T) {
	for _, change := range []string{"none", "revoke", "model_gate", "owner_disabled"} {
		t.Run(change, func(t *testing.T) {
			token := strings.Repeat("a", 48)
			store := &profileLiveStore{share: &profileShare{UserID: 52, Token: token, ModelUsageEnabled: true, AggregateUsageEnabled: true, LinkedProfiles: []LinkedProfile{}}}
			db := sql.OpenDB(store)
			defer db.Close()
			var readUsage atomic.Bool
			var usageCalls atomic.Int32
			core := profileLiveCore(t, func(w http.ResponseWriter, r *http.Request) {
				if strings.HasSuffix(r.URL.Path, "/users") {
					_ = json.NewEncoder(w).Encode([]CoreUserInfo{{UserID: 52, Active: change != "owner_disabled" || !readUsage.Load()}})
					return
				}
				var in map[string]int64
				_ = json.NewDecoder(r.Body).Decode(&in)
				if in["user_id"] != 52 {
					t.Error("owner not bound to original share")
				}
				usageCalls.Add(1)
				readUsage.Store(true)
				store.Lock()
				if change == "revoke" {
					store.share = nil
				}
				if change == "model_gate" {
					store.share.ModelUsageEnabled = false
				}
				store.Unlock()
				_ = json.NewEncoder(w).Encode(CoreUserUsage{UserID: 52, Active: true, Name: "Owner", CreditsPerUSD: 500000, Totals: CoreUsageTotals{Tokens: 1234, Requests: 7}, ModelCount: 1, Models: []CoreModelUsage{{ModelName: "real-core-model", Tokens: 1234, Requests: 7}}})
			})
			h := &handlers{s: &Service{db: db, core: core, now: func() time.Time { return time.Date(2026, 11, 10, 12, 0, 0, 0, time.UTC) }}}
			mux := http.NewServeMux()
			h.s.Register(mux, fakeAuth{})
			response := httptest.NewRecorder()
			mux.ServeHTTP(response, httptest.NewRequest("GET", "/api/share/profile/"+token+".svg?layout=models", nil))
			if change == "none" {
				if response.Code != 200 || !strings.Contains(response.Body.String(), "real-core-model") || response.Header().Get("Cache-Control") != "no-store" || response.Header().Get("Content-Type") != "image/svg+xml; charset=utf-8" {
					t.Fatalf("response %d %s", response.Code, response.Body.String())
				}
			} else if response.Code != 404 || strings.Contains(response.Body.String(), "real-core-model") {
				t.Fatalf("revoked output escaped: %d", response.Code)
			}
			if usageCalls.Load() != 1 {
				t.Fatal("unexpected usage read count")
			}
			store.Lock()
			store.share = &profileShare{UserID: 52, Token: token}
			store.Unlock()
			response = httptest.NewRecorder()
			mux.ServeHTTP(response, httptest.NewRequest("GET", "/api/share/profile/"+token+".svg?layout=models", nil))
			if response.Code != 404 || usageCalls.Load() != 1 {
				t.Fatal("disabled model consent fetched data")
			}
		})
	}
}

func TestProfileShareLiveRejectsCrossOwnerCoreReply(t *testing.T) {
	core := profileLiveCore(t, func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(CoreUserUsage{UserID: 99, Active: true, CreditsPerUSD: 500000})
	})
	if _, err := core.profileShareUsage(context.Background(), 52, 1, 2, 0, 0); err == nil {
		t.Fatal("cross-owner reply accepted")
	}
}

func TestProfileShareLiveHTTPMixedPricingKeepsUsage(t *testing.T) {
	for _, known := range []bool{false, true} {
		t.Run(map[bool]string{false: "mixed", true: "known"}[known], func(t *testing.T) {
			token := strings.Repeat("b", 48)
			store := &profileLiveStore{share: &profileShare{UserID: 52, Token: token, ModelUsageEnabled: true, LinkedProfiles: []LinkedProfile{}}}
			db := sql.OpenDB(store)
			defer db.Close()
			core := profileLiveCore(t, func(w http.ResponseWriter, r *http.Request) {
				if strings.HasSuffix(r.URL.Path, "/users") {
					_ = json.NewEncoder(w).Encode([]CoreUserInfo{{UserID: 52, Active: true}})
					return
				}
				var in map[string]int64
				_ = json.NewDecoder(r.Body).Decode(&in)
				if in["user_id"] != 52 || in["model_limit"] != 6 {
					t.Error("unscoped models")
				}
				var total, unknown *int64
				if known {
					total = profileLiveInt(1000000)
					unknown = profileLiveInt(500000)
				}
				out := map[string]any{"user_id": 52, "active": true, "credits_per_usd": 500000, "pricing_available": known, "cny_per_usd": "7.125", "model_count": 2,
					"totals": map[string]any{"tokens": 1000, "requests": 10, "credits": total},
					"models": []map[string]any{{"model_name": "priced-model", "tokens": 600, "requests": 6, "credits": 500000}, {"model_name": "unpriced-model", "tokens": 400, "requests": 4, "credits": unknown}}}
				_ = json.NewEncoder(w).Encode(out)
			})
			svc := &Service{db: db, core: core, now: func() time.Time { return time.Date(2026, 11, 10, 12, 0, 0, 0, time.UTC) }}
			mux := http.NewServeMux()
			svc.Register(mux, fakeAuth{})
			response := httptest.NewRecorder()
			mux.ServeHTTP(response, httptest.NewRequest("GET", "/api/share/profile/"+token+".svg?layout=models&currency=USD&format=full", nil))
			body := response.Body.String()
			if response.Code != 200 || !strings.Contains(body, "priced-model") || !strings.Contains(body, "unpriced-model") || !strings.Contains(body, "1000") || !strings.Contains(body, "1.0000 USD") {
				t.Fatalf("mixed usage lost: %d %s", response.Code, body)
			}
			if !known && (!strings.Contains(body, "Tokens by model") || strings.Contains(body, "0.0000 USD")) {
				t.Fatal("unknown became zero or cost share")
			}
			if known && (!strings.Contains(body, "Spend by model") || !strings.Contains(body, "2.0000 USD")) {
				t.Fatal("known total lost")
			}
			credit := httptest.NewRecorder()
			mux.ServeHTTP(credit, httptest.NewRequest("GET", "/api/share/profile/"+token+".svg?layout=models&currency=CREDIT&format=full", nil))
			creditBody := credit.Body.String()
			if credit.Code != 200 || !strings.Contains(creditBody, "500000 Credit") || strings.Contains(creditBody, " USD") || strings.Contains(creditBody, " CNY") {
				t.Fatalf("public Credit incorrectly converted: %d %s", credit.Code, creditBody)
			}
			if known && !strings.Contains(creditBody, "1000000 Credit") {
				t.Fatal("known credit total lost")
			}
			if !known && (!strings.Contains(creditBody, "Tokens by model") || strings.Contains(creditBody, ">0 Credit<")) {
				t.Fatal("unknown credit total became zero/cost share")
			}
		})
	}
}

func TestProfileShareLiveHTTPAllNativeLayouts(t *testing.T) {
	now := time.Date(2026, 11, 10, 12, 0, 0, 0, time.UTC)
	token := strings.Repeat("c", 48)
	store := &profileLiveStore{share: &profileShare{UserID: 52, Token: token, AggregateUsageEnabled: true, LinkedProfiles: []LinkedProfile{}}}
	db := sql.OpenDB(store)
	defer db.Close()
	var allTime atomic.Int32
	core := profileLiveCore(t, func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/users") {
			_ = json.NewEncoder(w).Encode([]CoreUserInfo{{UserID: 52, Active: true}})
			return
		}
		var in map[string]int64
		_ = json.NewDecoder(r.Body).Decode(&in)
		if in["user_id"] != 52 {
			t.Error("wrong owner")
		}
		if in["start"] == 0 {
			allTime.Add(1)
			if in["bucket_seconds"] != 0 {
				t.Error("alltime requested day buckets")
			}
		}
		out := CoreUserUsage{UserID: 52, Active: true, Name: "Native Owner", Username: "owner", PlatformRole: "admin", CreditsPerUSD: 500000, Totals: CoreUsageTotals{Tokens: 1234, Requests: 7, Credits: profileLiveInt(0)}, ModelCount: 1}
		if in["bucket_seconds"] == 86400 {
			out.Buckets = []CoreUsageBucket{{Bucket: now.UTC().Truncate(24 * time.Hour).Unix(), Tokens: 1234, Requests: 7}}
		}
		_ = json.NewEncoder(w).Encode(out)
	})
	svc := &Service{db: db, core: core, now: func() time.Time { return now }}
	mux := http.NewServeMux()
	svc.Register(mux, fakeAuth{})
	for _, query := range []string{"?format=full", "?layout=badge&period=all&format=full", "?layout=aggregate&period=all&format=full"} {
		response := httptest.NewRecorder()
		mux.ServeHTTP(response, httptest.NewRequest("GET", "/api/share/profile/"+token+".svg"+query, nil))
		if response.Code != 200 || !strings.Contains(response.Body.String(), "1234") {
			t.Fatalf("native layout %s: %d %s", query, response.Code, response.Body.String())
		}
		if strings.Contains(response.Body.String(), "credits") || strings.Contains(response.Body.String(), "model_name") {
			t.Fatal("private fields escaped non-model card")
		}
	}
	if allTime.Load() != 3 {
		t.Fatalf("alltime never read from Core: %d", allTime.Load())
	}
}

func TestProfileShareLiveSourcesConcurrent(t *testing.T) {
	testProfileShareLiveSourceGates(t, false)
}

func TestProfileShareLiveSourcesCancellation(t *testing.T) {
	testProfileShareLiveSourceGates(t, true)
}

func testProfileShareLiveSourceGates(t *testing.T, cancelRequest bool) {
	t.Helper()
	now := time.Date(2026, 10, 4, 12, 0, 0, 0, time.UTC)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	coreEntered, cursorEntered := make(chan context.Context, 1), make(chan context.Context, 1)
	coreCanceled, cursorCanceled := make(chan struct{}), make(chan struct{})
	release := make(chan struct{})
	var releaseOnce sync.Once
	finishReads := func() { releaseOnce.Do(func() { close(release) }) }
	gate := func(entered chan<- context.Context, canceled chan struct{}, body, contentType string) profileLiveRoundTrip {
		return func(r *http.Request) (*http.Response, error) {
			entered <- r.Context()
			select {
			case <-r.Context().Done():
				close(canceled)
				return nil, r.Context().Err()
			case <-release:
				return &http.Response{StatusCode: http.StatusOK, Header: http.Header{"Content-Type": {contentType}, "Cache-Control": {"no-store"}}, Body: io.NopCloser(strings.NewReader(body)), Request: r}, nil
			}
		}
	}
	core := &CoreRead{token: "synthetic-profile-share-purpose-token", client: &http.Client{Timeout: 12 * time.Second, Transport: gate(coreEntered, coreCanceled, `{"user_id":52,"active":true,"credits_per_usd":500000,"model_count":0,"totals":{"tokens":1234,"requests":7,"credits":null}}`, "application/json")}}
	previousClient := profileShareHTTPClient
	profileShareHTTPClient = &http.Client{Timeout: 4 * time.Second, Transport: gate(cursorEntered, cursorCanceled, profileLiveCursorHTML, "text/html")}
	profileCursorCache.Lock()
	previousCache := profileCursorCache.Entries
	profileCursorCache.Entries = make(map[string]profileCursorCacheEntry)
	profileCursorCache.Unlock()
	done := make(chan struct{})
	var rows []aggregateSource
	t.Cleanup(func() {
		cancel()
		finishReads()
		// Collect all workers before restoring global test resources.
		select {
		case <-done:
		case <-time.After(2 * time.Second):
			t.Error("source resolver did not finish during cleanup")
			return
		}
		profileShareHTTPClient = previousClient
		profileCursorCache.Lock()
		profileCursorCache.Entries = previousCache
		profileCursorCache.Unlock()
	})
	s := &Service{core: core}
	share := &profileShare{UserID: 52, AggregateUsageEnabled: true, LinkedProfiles: []LinkedProfile{{Provider: "cursor", URL: "https://cursor.com/@profile-fixture", Label: "Public Cursor"}}}
	go func() {
		rows = s.resolveProfileShareSources(ctx, share, "all", now)
		close(done)
	}()
	await := func(ch <-chan struct{}, message string) {
		t.Helper()
		select {
		case <-ch:
		case <-time.After(2 * time.Second):
			t.Fatal(message)
		}
	}
	var coreContext context.Context
	select {
	case coreContext = <-coreEntered:
	case <-time.After(2 * time.Second):
		t.Fatal("Core did not start")
	}
	// Inspect the client's local transport context; HTTP servers cannot see
	// the originating client's deadline. No real five-second wait is needed.
	if deadline, ok := coreContext.Deadline(); !ok || time.Until(deadline) <= 5*time.Second {
		t.Fatal("Core was given the shared five-second provider deadline")
	}
	select {
	case <-cursorEntered:
	case <-time.After(2 * time.Second):
		t.Fatal("Cursor did not start while Core was still blocked")
	}
	if cancelRequest {
		cancel()
		await(coreCanceled, "Core did not observe original request cancellation")
		await(cursorCanceled, "Cursor did not observe original request cancellation")
	} else {
		finishReads()
	}
	await(done, "source resolver did not finish")
	if len(rows) != 2 || rows[1].Label != "Public Cursor" {
		t.Fatalf("source order or label changed: %#v", rows)
	}
	if cancelRequest {
		if rows[0].Status != "unavailable" || rows[1].Status != "unavailable" || rows[0].Tokens != nil || rows[1].Tokens != nil {
			t.Fatalf("cancellation became successful or zero-valued usage: %#v", rows)
		}
	} else if rows[0].Status != "live" || rows[0].Tokens == nil || *rows[0].Tokens != 1234 || rows[1].Status != "live" || rows[1].Tokens == nil || *rows[1].Tokens <= 0 || rows[1].Source != "public_ssr" || rows[1].Period != "reported" {
		t.Fatalf("independent source results lost: %#v", rows)
	}
}
