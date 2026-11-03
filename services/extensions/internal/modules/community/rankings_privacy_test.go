// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestUserRankingsObservePrivacyChanges(t *testing.T) {
	anonymous, unavailable := false, false
	calls := 0
	authority := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.URL.Path != "/internal/community/v1/usage/users" {
			t.Errorf("unexpected Core route %s", r.URL.Path)
		}
		if unavailable {
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		count := int64(0)
		if anonymous {
			count = 1
		}
		// Deliberately retain the fixture name: the public adapter must suppress it.
		_ = json.NewEncoder(w).Encode(CoreUserRankings{TotalTokens: 100, TotalRequests: 1, ParticipantCount: 1, AnonymousParticipantCount: count, Users: []CoreRankedUser{{UserID: 7, Name: "synthetic-public-name", Anonymous: anonymous, Tokens: 100, Requests: 1}}})
	}))
	defer authority.Close()
	client := authority.Client()
	// CoreRead uses a fixed internal hostname; direct this test transport to the fixture.
	client.Transport = rankingFixtureTransport{base: client.Transport, target: authority.URL}
	service := &Service{core: &CoreRead{client: client, token: "synthetic-purpose-token"}, now: func() time.Time { return time.Unix(1793740000, 0) }}
	h := &handlers{s: service}
	call := func() *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		h.userRankings(w, httptest.NewRequest("GET", "/api/rankings/users?period=month", nil))
		return w
	}
	decode := func(w *httptest.ResponseRecorder) UserUsageRankingsResponse {
		t.Helper()
		var result struct {
			Success bool
			Data    UserUsageRankingsResponse
		}
		if w.Code != 200 || json.Unmarshal(w.Body.Bytes(), &result) != nil || !result.Success || len(result.Data.Users) != 1 {
			t.Fatalf("unexpected ranking response: HTTP %d", w.Code)
		}
		return result.Data
	}
	first := decode(call())
	if first.Users[0].Name != "synthetic-public-name" || first.Users[0].Anonymous {
		t.Fatal("public opt-in name missing")
	}
	anonymous = true
	secondResponse := call()
	second := decode(secondResponse)
	if !second.Users[0].Anonymous || second.Users[0].Name != "" || second.AnonymousParticipantCount != 1 {
		t.Fatal("withdrawn public name survives in ranking cache")
	}
	if secondResponse.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("privacy-bearing ranking response permits caching")
	}
	unavailable = true
	if w := call(); w.Code != 503 {
		t.Fatalf("Core unavailable must not replay previous identity: HTTP %d", w.Code)
	}
	if calls != 3 {
		t.Fatalf("each public ranking request must consult current Core privacy; got %d reads", calls)
	}
}

type rankingFixtureTransport struct {
	base   http.RoundTripper
	target string
}

func (t rankingFixtureTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	copy := r.Clone(r.Context())
	u := *r.URL
	copy.URL = &u
	copy.URL.Scheme = "http"
	copy.URL.Host = strings.TrimPrefix(t.target, "http://")
	return t.base.RoundTrip(copy)
}
