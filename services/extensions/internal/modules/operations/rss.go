// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"regexp"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"golang.org/x/sync/singleflight"
)

const MaxRSSFeeds = 24
const maxRSSBytes = 2 << 20
const rssConcurrency = 6

var rssID = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)
var ErrConflict = errors.New("operations configuration changed")

type RSSFeedConfig struct {
	ID      string `json:"id"`
	Name    string `json:"name"`
	URL     string `json:"url"`
	Enabled bool   `json:"enabled"`
}

type RSSSettings struct {
	Revision int64           `json:"revision"`
	Feeds    []RSSFeedConfig `json:"feeds"`
}

type RSSItem struct {
	ID          string `json:"id"`
	Title       string `json:"title"`
	URL         string `json:"url"`
	Summary     string `json:"summary"`
	PublishedAt string `json:"published_at,omitempty"`
}

type RSSFeed struct {
	ID    string    `json:"id"`
	Name  string    `json:"name"`
	URL   string    `json:"url"`
	Items []RSSItem `json:"items"`
	Error string    `json:"error,omitempty"`
}

type RSS struct {
	db      *sql.DB
	client  *http.Client
	permits chan struct{}
	group   singleflight.Group
	mu      sync.Mutex
	key     string
	expires time.Time
	feeds   []RSSFeed
}

func newRSS(db *sql.DB) *RSS {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	transport.MaxConnsPerHost = rssConcurrency
	transport.MaxIdleConns = rssConcurrency
	transport.MaxIdleConnsPerHost = 2
	transport.ResponseHeaderTimeout = 6 * time.Second
	transport.DialContext = rssDial
	client := &http.Client{Transport: transport, Timeout: 6 * time.Second, CheckRedirect: func(r *http.Request, via []*http.Request) error {
		if len(via) >= 3 {
			return ErrInvalid
		}
		_, err := rssURL(r.URL.String())
		return err
	}}
	return &RSS{db: db, client: client, permits: make(chan struct{}, rssConcurrency)}
}

func rssPublicIP(ip netip.Addr) bool {
	ip = ip.Unmap()
	if !ip.IsValid() || !ip.IsGlobalUnicast() || ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast() {
		return false
	}
	for _, block := range []string{"100.64.0.0/10", "192.0.0.0/24", "198.18.0.0/15", "64:ff9b::/96", "64:ff9b:1::/48"} {
		if netip.MustParsePrefix(block).Contains(ip) {
			return false
		}
	}
	return true
}

func rssURL(raw string) (*url.URL, error) {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || len(raw) > 2048 || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" || u.User != nil || u.Opaque != "" {
		return nil, ErrInvalid
	}
	host := strings.TrimSuffix(strings.ToLower(u.Hostname()), ".")
	if host == "localhost" || strings.HasSuffix(host, ".localhost") || strings.HasSuffix(host, ".local") {
		return nil, ErrInvalid
	}
	if ip, err := netip.ParseAddr(host); err == nil && !rssPublicIP(ip) {
		return nil, ErrInvalid
	}
	u.Fragment = ""
	return u, nil
}

func rssDial(ctx context.Context, network, address string) (net.Conn, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, ErrInvalid
	}
	ips, err := net.DefaultResolver.LookupNetIP(ctx, "ip", host)
	if err != nil || len(ips) == 0 {
		return nil, ErrUnavailable
	}
	for _, ip := range ips {
		if !rssPublicIP(ip) {
			return nil, ErrInvalid
		}
	}
	return (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, network, net.JoinHostPort(ips[0].String(), port))
}

func validateRSS(feeds []RSSFeedConfig) ([]RSSFeedConfig, error) {
	if feeds == nil || len(feeds) > MaxRSSFeeds {
		return nil, ErrInvalid
	}
	result := make([]RSSFeedConfig, 0, len(feeds))
	ids, urls := map[string]bool{}, map[string]bool{}
	for _, feed := range feeds {
		feed.ID, feed.Name = strings.TrimSpace(feed.ID), strings.TrimSpace(feed.Name)
		u, err := rssURL(feed.URL)
		if err != nil || !rssID.MatchString(feed.ID) || ids[feed.ID] || !utf8.ValidString(feed.Name) || feed.Name == "" || utf8.RuneCountInString(feed.Name) > 80 || urls[u.String()] {
			return nil, ErrInvalid
		}
		feed.URL = u.String()
		ids[feed.ID], urls[feed.URL] = true, true
		result = append(result, feed)
	}
	return result, nil
}

func (s *RSS) Settings(ctx context.Context) (RSSSettings, error) {
	if s == nil || s.db == nil {
		return RSSSettings{}, ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var settings RSSSettings
	var raw []byte
	if err := s.db.QueryRowContext(ctx, `SELECT revision,feeds FROM lmm_operations.rss_config WHERE singleton`).Scan(&settings.Revision, &raw); err != nil || json.Unmarshal(raw, &settings.Feeds) != nil {
		return RSSSettings{}, ErrUnavailable
	}
	if _, err := validateRSS(settings.Feeds); err != nil {
		return RSSSettings{}, ErrUnavailable
	}
	return settings, nil
}

func (s *RSS) Save(ctx context.Context, actor int64, settings RSSSettings) (RSSSettings, error) {
	if s == nil || s.db == nil {
		return RSSSettings{}, ErrUnavailable
	}
	feeds, err := validateRSS(settings.Feeds)
	if err != nil || !validID(actor) || settings.Revision < 0 || settings.Revision >= MaxID {
		return RSSSettings{}, ErrInvalid
	}
	raw, _ := json.Marshal(feeds)
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	err = s.db.QueryRowContext(ctx, `UPDATE lmm_operations.rss_config SET feeds=$1,revision=revision+1,updated_by=$2 WHERE singleton AND revision=$3 RETURNING revision`, string(raw), actor, settings.Revision).Scan(&settings.Revision)
	if errors.Is(err, sql.ErrNoRows) {
		return RSSSettings{}, ErrConflict
	}
	if err != nil {
		return RSSSettings{}, ErrUnavailable
	}
	settings.Feeds = feeds
	return settings, nil
}

func (s *RSS) Read(ctx context.Context) ([]RSSFeed, error) {
	settings, err := s.Settings(ctx)
	if err != nil {
		return nil, err
	}
	return s.load(ctx, settings.Feeds)
}

func (s *RSS) load(ctx context.Context, configs []RSSFeedConfig) ([]RSSFeed, error) {
	raw, _ := json.Marshal(configs)
	key := string(raw)
	result := s.group.DoChan(key, func() (any, error) {
		s.mu.Lock()
		if s.key == key && time.Now().Before(s.expires) {
			feeds := s.feeds
			s.mu.Unlock()
			return feeds, nil
		}
		s.mu.Unlock()
		work, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		var enabled []RSSFeedConfig
		for _, config := range configs {
			if config.Enabled {
				enabled = append(enabled, config)
			}
		}
		feeds := make([]RSSFeed, len(enabled))
		var wg sync.WaitGroup
		for i, config := range enabled {
			wg.Go(func() { feeds[i] = s.fetch(work, config) })
		}
		wg.Wait()
		ttl := 3 * time.Minute
		for _, feed := range feeds {
			if feed.Error != "" {
				ttl = time.Minute
			}
		}
		s.mu.Lock()
		s.key, s.expires, s.feeds = key, time.Now().Add(ttl), feeds
		s.mu.Unlock()
		return feeds, nil
	})
	select {
	case <-ctx.Done():
		return nil, ErrUnavailable
	case r := <-result:
		// The cached slices are never returned to callers for mutation.
		feeds := r.Val.([]RSSFeed)
		copy := make([]RSSFeed, len(feeds))
		for i, feed := range feeds {
			copy[i] = feed
			copy[i].Items = append([]RSSItem{}, feed.Items...)
		}
		return copy, nil
	}
}

func (s *RSS) fetch(ctx context.Context, config RSSFeedConfig) RSSFeed {
	result := RSSFeed{ID: config.ID, Name: config.Name, URL: config.URL, Items: []RSSItem{}}
	fail := func(code string) RSSFeed { result.Error = code; return result }
	if _, err := rssURL(config.URL); err != nil {
		return fail("blocked")
	}
	select {
	case s.permits <- struct{}{}:
		defer func() { <-s.permits }()
	case <-ctx.Done():
		return fail("timeout")
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, config.URL, nil)
	if err != nil {
		return fail("invalid")
	}
	req.Header.Set("Accept", "application/rss+xml, application/atom+xml, application/xml, text/xml")
	req.Header.Set("User-Agent", "LMM API RSS reader")
	res, err := s.client.Do(req)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) {
			return fail("timeout")
		}
		if errors.Is(err, ErrInvalid) {
			return fail("blocked")
		}
		return fail("unavailable")
	}
	defer res.Body.Close()
	if res.StatusCode < 200 || res.StatusCode >= 300 {
		return fail("http")
	}
	if res.ContentLength > maxRSSBytes {
		return fail("too_large")
	}
	body, err := io.ReadAll(io.LimitReader(res.Body, maxRSSBytes+1))
	if err != nil {
		return fail("unavailable")
	}
	if len(body) > maxRSSBytes {
		return fail("too_large")
	}
	result.Items, err = parseRSS(body, res.Request.URL.String())
	if err != nil {
		result.Items = []RSSItem{}
		return fail("invalid")
	}
	return result
}
