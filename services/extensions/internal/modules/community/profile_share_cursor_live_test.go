// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestProfileShareLiveCursorPublicSSR(t *testing.T) {
	now := time.Date(2026, 10, 4, 12, 0, 0, 0, time.UTC)
	row, err := parseProfileCursorHTML([]byte(profileLiveCursorHTML), "profile-fixture", now)
	if err != nil || row.Status != "live" || row.Tokens == nil || *row.Tokens <= 0 || row.Requests != nil || row.Messages != nil || row.Period != "reported" || row.PeriodTimezone != "unspecified" || row.Source != "public_ssr" {
		t.Fatalf("SSR: %#v %v", row, err)
	}
	for _, body := range []string{
		strings.ReplaceAll(profileLiveCursorHTML, "PUBLIC", "PRIVATE"),
		strings.ReplaceAll(profileLiveCursorHTML, "external", "owner"),
		strings.ReplaceAll(profileLiveCursorHTML, "2026-09-04", "2026-09-06"),
		strings.ReplaceAll(profileLiveCursorHTML, "115326150", "9007199254740992"),
		profileLiveCursorHTML + profileLiveCursorHTML,
		strings.Repeat("x", profileShareCursorMaxBody+1),
	} {
		if _, e := parseProfileCursorHTML([]byte(body), "profile-fixture", now); e == nil {
			t.Fatal("invalid/private SSR accepted")
		}
	}
	if _, e := parseProfileCursorHTML([]byte(profileLiveCursorHTML), "another-owner", now); e == nil {
		t.Fatal("wrong profile accepted")
	}
}

func profileLiveCursorClient(t *testing.T, handler http.HandlerFunc) {
	t.Helper()
	srv := httptest.NewServer(handler)
	t.Cleanup(srv.Close)
	dest, _ := url.Parse(srv.URL)
	previous := profileShareHTTPClient
	profileShareHTTPClient = &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }, Transport: profileLiveRoundTrip(func(r *http.Request) (*http.Response, error) {
		if r.URL.Host != "cursor.com" || r.Header.Get("Authorization") != "" || r.Header.Get("Cookie") != "" {
			t.Error("nonanonymous or wrong destination")
		}
		clone := r.Clone(r.Context())
		u := *r.URL
		clone.URL = &u
		clone.URL.Scheme, clone.URL.Host = dest.Scheme, dest.Host
		return http.DefaultTransport.RoundTrip(clone)
	})}
	profileCursorCache.Lock()
	profileCursorCache.Entries = make(map[string]profileCursorCacheEntry)
	profileCursorCache.Unlock()
	t.Cleanup(func() {
		profileShareHTTPClient = previous
		profileCursorCache.Lock()
		profileCursorCache.Entries = make(map[string]profileCursorCacheEntry)
		profileCursorCache.Unlock()
	})
}

func TestProfileShareLiveCursorCacheAndPrivacy(t *testing.T) {
	now := time.Date(2026, 10, 4, 12, 0, 0, 0, time.UTC)
	var requests atomic.Int32
	var private atomic.Bool
	var noStore atomic.Bool
	profileLiveCursorClient(t, func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.Method != "GET" {
			t.Error("unexpected method")
		}
		if noStore.Load() {
			w.Header().Set("Cache-Control", "no-store")
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if private.Load() {
			w.WriteHeader(403)
			return
		}
		_, _ = w.Write([]byte(profileLiveCursorHTML))
	})
	raw := "https://cursor.com/@profile-fixture"
	if row := fetchProfileCursor(context.Background(), raw, now); row.Status != "live" {
		t.Fatal(row)
	}
	if row := fetchProfileCursor(context.Background(), raw, now.Add(time.Second)); row.Status != "live" || requests.Load() != 1 {
		t.Fatal("missing positive cache")
	}
	private.Store(true)
	if row := fetchProfileCursor(context.Background(), raw, now.Add(61*time.Second)); row.Status != "unavailable" || row.Tokens != nil || requests.Load() != 2 {
		t.Fatal("private/error response retained stale metrics")
	}
	private.Store(false)
	noStore.Store(true)
	if row := fetchProfileCursor(context.Background(), raw, now.Add(80*time.Second)); row.Status != "live" {
		t.Fatal(row)
	}
	profileCursorCache.Lock()
	_, cached := profileCursorCache.Entries[raw]
	profileCursorCache.Unlock()
	if cached {
		t.Fatal("no-store source cached")
	}
	private.Store(true)
	if row := fetchProfileCursor(context.Background(), raw, now.Add(81*time.Second)); row.Tokens != nil {
		t.Fatal("no-store private metrics retained")
	}
}

func TestProfileShareLiveCursorNetworkBoundary(t *testing.T) {
	for _, raw := range []string{"127.0.0.1", "10.0.0.1", "169.254.169.254", "100.64.0.1", "192.0.2.1", "::1", "::ffff:127.0.0.1", "2001:db8::1"} {
		if profileSharePublicIP(netip.MustParseAddr(raw)) {
			t.Fatalf("unsafe IP accepted: %s", raw)
		}
	}
	if !profileSharePublicIP(netip.MustParseAddr("1.1.1.1")) {
		t.Fatal("public IP rejected")
	}
	if _, err := profileShareSafeDial(context.Background(), "tcp", "example.com:443"); err == nil {
		t.Fatal("arbitrary hostname accepted")
	}
	var requests atomic.Int32
	profileLiveCursorClient(t, func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.URL.Path == "/@oversized" {
			w.Header().Set("Content-Type", "text/html")
			_, _ = w.Write([]byte(strings.Repeat("x", profileShareCursorMaxBody+1)))
			return
		}
		http.Redirect(w, r, "http://127.0.0.1/secret", http.StatusFound)
	})
	now := time.Date(2026, 10, 4, 12, 0, 0, 0, time.UTC)
	for _, raw := range []string{"https://cursor.com/@redirect", "https://cursor.com/@oversized"} {
		if row := fetchProfileCursor(context.Background(), raw, now); row.Status != "unavailable" || row.Tokens != nil {
			t.Fatal(row)
		}
	}
	for _, raw := range []string{"https://cursor.com.evil.test/@profile-fixture", "https://cursor.com/@a?cookie=secret", "http://cursor.com/@a", "https://127.0.0.1/@a"} {
		if row := fetchProfileCursor(context.Background(), raw, now); row.Reason != "invalid_profile_url" {
			t.Fatal(row)
		}
	}
	if requests.Load() != 2 {
		t.Fatal("unsafe URL fetched or redirect followed")
	}
}

// Fixed public SSR fixture from cc52aa195c6f2a01d703a1a407be73c31b89867d; not live provider acceptance.
const profileLiveCursorHTML = "<!doctype html><html><body><script>self.__next_f.push([1,\"25:[\\\"$\\\",\\\"$L51\\\",null,{\\\"profile\\\":{\\\"visibility\\\":\\\"PUBLIC\\\",\\\"viewedAs\\\":\\\"external\\\",\\\"stats\\\":{\\\"mostActiveMonth\\\":\\\"August\\\",\\\"mostActiveDay\\\":\\\"Aug 23\\\",\\\"longestStreak\\\":11,\\\"currentStreak\\\":11,\\\"agentsLocal\\\":30,\\\"agentsCloud\\\":28,\\\"longestAgentSeconds\\\":27801},\\\"tokensOverTime\\\":[{\\\"date\\\":\\\"2026-09-04\\\",\\\"tokens\\\":115326150},{\\\"date\\\":\\\"2026-09-05\\\",\\\"tokens\\\":56532391},{\\\"date\\\":\\\"2026-09-06\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-07\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-08\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-09\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-10\\\",\\\"tokens\\\":24455},{\\\"date\\\":\\\"2026-09-11\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-12\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-13\\\",\\\"tokens\\\":24424},{\\\"date\\\":\\\"2026-09-14\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-15\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-16\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-17\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-18\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-19\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-20\\\",\\\"tokens\\\":48295},{\\\"date\\\":\\\"2026-09-21\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-22\\\",\\\"tokens\\\":0},{\\\"date\\\":\\\"2026-09-23\\\",\\\"tokens\\\":17009228},{\\\"date\\\":\\\"2026-09-24\\\",\\\"tokens\\\":20237668},{\\\"date\\\":\\\"2026-09-25\\\",\\\"tokens\\\":10108278},{\\\"date\\\":\\\"2026-09-26\\\",\\\"tokens\\\":16709875},{\\\"date\\\":\\\"2026-09-27\\\",\\\"tokens\\\":15060752},{\\\"date\\\":\\\"2026-09-28\\\",\\\"tokens\\\":108841427},{\\\"date\\\":\\\"2026-09-29\\\",\\\"tokens\\\":89090885},{\\\"date\\\":\\\"2026-09-30\\\",\\\"tokens\\\":23922528},{\\\"date\\\":\\\"2026-10-01\\\",\\\"tokens\\\":11685578},{\\\"date\\\":\\\"2026-10-02\\\",\\\"tokens\\\":6908612},{\\\"date\\\":\\\"2026-10-03\\\",\\\"tokens\\\":11308827}],\\\"agentsOverTime\\\":[{\\\"date\\\":\\\"2026-09-04\\\",\\\"local\\\":3,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-05\\\",\\\"local\\\":2,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-06\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-07\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-08\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-09\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-10\\\",\\\"local\\\":0,\\\"cloud\\\":1},{\\\"date\\\":\\\"2026-09-11\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-12\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-13\\\",\\\"local\\\":0,\\\"cloud\\\":1},{\\\"date\\\":\\\"2026-09-14\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-15\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-16\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-17\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-18\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-19\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-20\\\",\\\"local\\\":0,\\\"cloud\\\":1},{\\\"date\\\":\\\"2026-09-21\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-22\\\",\\\"local\\\":0,\\\"cloud\\\":0},{\\\"date\\\":\\\"2026-09-23\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-24\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-25\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-26\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-27\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-28\\\",\\\"local\\\":19,\\\"cloud\\\":3},{\\\"date\\\":\\\"2026-09-29\\\",\\\"local\\\":7,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-09-30\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-10-01\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-10-02\\\",\\\"local\\\":0,\\\"cloud\\\":2},{\\\"date\\\":\\\"2026-10-03\\\",\\\"local\\\":0,\\\"cloud\\\":2}],\\\"dailyAgentCounts\\\":[],\\\"topModels\\\":[{\\\"name\\\":\\\"Composer 2.5\\\",\\\"vendor\\\":\\\"cursor\\\",\\\"agentRequests\\\":20}],\\\"handle\\\":\\\"profile-fixture\\\"}}]\\n\"]);</script></body></html>\n"
