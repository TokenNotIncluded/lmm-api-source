// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestRSSFormatsAndPublicProjection(t *testing.T) {
	for _, document := range []string{
		`<rss><channel><item><title>Title</title><link>/article</link><guid>one</guid><description>&lt;b&gt;Plain&lt;/b&gt; &amp; safe</description><pubDate>Mon, 09 Nov 2026 12:00:00 +0000</pubDate></item></channel></rss>`,
		`<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><item><title>Title</title><link>/article</link><guid>one</guid><description>&lt;b&gt;Plain&lt;/b&gt; &amp; safe</description><date>2026-11-09T12:00:00Z</date></item></rdf:RDF>`,
		`<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>Title</title><link href="/article"/><id>one</id><summary>&lt;b&gt;Plain&lt;/b&gt; &amp; safe</summary><updated>2026-11-09T12:00:00Z</updated></entry></feed>`,
	} {
		items, err := parseRSS([]byte(document), "https://feeds.example/news.xml")
		if err != nil || len(items) != 1 || items[0] != (RSSItem{ID: "one", Title: "Title", URL: "https://feeds.example/article", Summary: "Plain & safe", PublishedAt: "2026-11-09T12:00:00Z"}) {
			t.Fatalf("feed projection: %#v %v", items, err)
		}
	}
	items, err := parseRSS([]byte(`<rss><channel><item><title>Untrusted</title><link>javascript:alert(1)</link><description><![CDATA[<img src=x onerror=alert(1)>Text]]></description></item></channel></rss>`), "https://feeds.example")
	if err != nil || len(items) != 1 || items[0].URL != "" || items[0].Summary != "Text" {
		t.Fatal(items, err)
	}
	if _, err := parseRSS([]byte(`<html><body>login page</body></html>`), "https://feeds.example"); err == nil {
		t.Fatal("HTML was accepted as an empty successful feed")
	}
	var many strings.Builder
	many.WriteString("<rss><channel>")
	for i := 0; i < 70; i++ {
		fmt.Fprintf(&many, "<item><title>item %d</title></item>", i)
	}
	many.WriteString("</channel></rss>")
	items, err = parseRSS([]byte(many.String()), "https://feeds.example")
	if err != nil || len(items) != 50 {
		t.Fatal("feed item bound", len(items), err)
	}
}

func TestRSSConfigurationAndNetworkBoundary(t *testing.T) {
	for _, raw := range []string{"file:///etc/passwd", "http://localhost/feed", "http://localhost./feed", "http://a.local/feed", "https://user:secret@feeds.example", "http://127.0.0.1", "http://[::1]", "http://169.254.169.254", "http://100.64.0.1", "http://10.0.0.1", "http://[::ffff:127.0.0.1]"} {
		if _, err := rssURL(raw); err == nil {
			t.Fatal("private or credentialed URL accepted", raw)
		}
	}
	for _, address := range []string{"127.0.0.1:80", "[::1]:80", "10.0.0.1:80", "169.254.169.254:80"} {
		if conn, err := rssDial(context.Background(), "tcp", address); err == nil || conn != nil {
			t.Fatal("private resolved address dialed", address)
		}
	}
	if !rssPublicIP(netip.MustParseAddr("1.1.1.1")) || !rssPublicIP(netip.MustParseAddr("2606:4700:4700::1111")) {
		t.Fatal("ordinary public addresses rejected")
	}
	base := RSSFeedConfig{ID: "news", Name: "News", URL: "https://feeds.example/feed", Enabled: true}
	if _, err := validateRSS([]RSSFeedConfig{base, base}); err == nil {
		t.Fatal("duplicate IDs accepted")
	}
	other := base
	other.ID, other.URL = "other", base.URL+"#fragment"
	if _, err := validateRSS([]RSSFeedConfig{base, other}); err == nil {
		t.Fatal("equivalent URLs accepted")
	}
	if _, err := validateRSS(nil); err == nil {
		t.Fatal("missing feeds accepted")
	}
}

func localRSS(t *testing.T, handler http.HandlerFunc) *RSS {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	s := newRSS(nil)
	transport := s.client.Transport.(*http.Transport)
	transport.DialContext = func(ctx context.Context, network, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
	}
	t.Cleanup(transport.CloseIdleConnections)
	return s
}

func TestRSSRealHTTPFetchCachingAndFailureStates(t *testing.T) {
	var calls atomic.Int32
	s := localRSS(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		switch r.URL.Path {
		case "/feed":
			fmt.Fprint(w, `<rss><channel><item><title>Fetched</title><link>/article</link></item></channel></rss>`)
		case "/redirect":
			http.Redirect(w, r, "http://127.0.0.1/private", http.StatusFound)
		case "/large":
			fmt.Fprint(w, strings.Repeat("x", maxRSSBytes+1))
		default:
			http.Error(w, "private supplier detail", http.StatusServiceUnavailable)
		}
	})
	config := []RSSFeedConfig{{ID: "one", Name: "One", URL: "http://feed.example/feed", Enabled: true}, {ID: "disabled", Name: "Disabled", URL: "http://feed.example/failure"}}
	var wg sync.WaitGroup
	for i := 0; i < 12; i++ {
		wg.Go(func() {
			feeds, err := s.load(context.Background(), config)
			if err != nil || len(feeds) != 1 || len(feeds[0].Items) != 1 || feeds[0].Items[0].Title != "Fetched" {
				t.Errorf("real HTTP fetch: %#v %v", feeds, err)
			}
		})
	}
	wg.Wait()
	if calls.Load() != 1 {
		t.Fatal("duplicate fetch or disabled feed dispatch", calls.Load())
	}
	feeds, _ := s.load(context.Background(), config)
	feeds[0].Items[0].Title = "mutated"
	feeds, _ = s.load(context.Background(), config)
	if feeds[0].Items[0].Title != "Fetched" {
		t.Fatal("caller mutated shared cache")
	}
	for path, expected := range map[string]string{"redirect": "blocked", "large": "too_large", "failure": "http"} {
		config[0].URL = "http://feed.example/" + path
		feeds, err := s.load(context.Background(), config)
		if err != nil || len(feeds) != 1 || feeds[0].Error != expected || len(feeds[0].Items) != 0 {
			t.Fatal(path, feeds, err)
		}
		before := calls.Load()
		_, _ = s.load(context.Background(), config)
		if calls.Load() != before {
			t.Fatal("failure not cached", path)
		}
	}
}

func TestRSSConcurrencyAndCallerCancellation(t *testing.T) {
	var active, maximum atomic.Int32
	started, release := make(chan struct{}, 24), make(chan struct{})
	s := localRSS(t, func(w http.ResponseWriter, r *http.Request) {
		now := active.Add(1)
		defer active.Add(-1)
		for old := maximum.Load(); now > old && !maximum.CompareAndSwap(old, now); old = maximum.Load() {
		}
		started <- struct{}{}
		select {
		case <-release:
			fmt.Fprint(w, `<feed/>`)
		case <-r.Context().Done():
		}
	})
	configs := make([]RSSFeedConfig, 12)
	for i := range configs {
		configs[i] = RSSFeedConfig{ID: fmt.Sprint(i), Name: "feed", URL: fmt.Sprintf("http://feed.example/%d", i), Enabled: true}
	}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { _, err := s.load(ctx, configs); done <- err }()
	for i := 0; i < rssConcurrency; i++ {
		select {
		case <-started:
		case <-time.After(2 * time.Second):
			t.Fatal("fetchers did not start")
		}
	}
	cancel()
	if err := <-done; err == nil {
		t.Fatal("cancelled caller received success")
	}
	close(release)
	if feeds, err := s.load(context.Background(), configs); err != nil || len(feeds) != 12 || maximum.Load() > rssConcurrency {
		t.Fatal("shared fetch/concurrency", feeds, err, maximum.Load())
	}
}
