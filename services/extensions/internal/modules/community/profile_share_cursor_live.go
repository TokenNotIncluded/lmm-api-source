// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Ported from TokenNotIncluded/lmm-api cc52aa195c6f2a01d703a1a407be73c31b89867d
// apps/api-go/controller; retains the fixed legacy public SSR parser and network boundary.
package community

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strings"
	"sync"
	"time"
)

const profileShareCursorMaxBody = 1 << 20

type profileCursorData struct {
	Handle     string `json:"handle"`
	Visibility string `json:"visibility"`
	ViewedAs   string `json:"viewedAs"`
	Stats      struct {
		AgentsLocal *int64 `json:"agentsLocal"`
		AgentsCloud *int64 `json:"agentsCloud"`
	} `json:"stats"`
	TokensOverTime []struct {
		Date   string `json:"date"`
		Tokens *int64 `json:"tokens"`
	} `json:"tokensOverTime"`
}

// This adapter reads JSON embedded in public SSR, never evaluates JavaScript.
// The 30 date labels are the reported window, with unspecified timezone; they
// must not be relabeled as LMM's rolling 30*24h window or all-time activity.
func parseProfileCursorHTML(body []byte, handle string, now time.Time) (aggregateSource, error) {
	var row aggregateSource
	if len(body) > profileShareCursorMaxBody {
		return row, errors.New("profile response too large")
	}
	var stream strings.Builder
	remaining := body
	const marker = "self.__next_f.push("
	pushes := 0
	for {
		index := bytes.Index(remaining, []byte(marker))
		if index < 0 {
			break
		}
		remaining = remaining[index+len(marker):]
		pushes++
		if pushes > 256 {
			return row, errors.New("too many profile records")
		}
		decoder := json.NewDecoder(bytes.NewReader(remaining))
		var args []json.RawMessage
		if err := decoder.Decode(&args); err != nil {
			return row, errors.New("invalid profile JSON chunk")
		}
		offset := int(decoder.InputOffset())
		if offset >= len(remaining) || remaining[offset] != ')' {
			return row, errors.New("invalid profile JSON push")
		}
		remaining = remaining[offset+1:]
		if len(args) != 2 || string(args[0]) != "1" {
			continue
		}
		var chunk string
		if err := json.Unmarshal(args[1], &chunk); err != nil {
			return row, errors.New("invalid profile JSON string")
		}
		stream.WriteString(chunk)
		if stream.Len() > profileShareCursorMaxBody {
			return row, errors.New("profile records too large")
		}
	}
	var candidates []map[string]any
	var findProfile func(any)
	findProfile = func(value any) {
		switch v := value.(type) {
		case []any:
			for _, child := range v {
				findProfile(child)
			}
		case map[string]any:
			if profile, ok := v["profile"].(map[string]any); ok {
				if _, ok := profile["tokensOverTime"]; ok {
					candidates = append(candidates, profile)
				}
			}
			for _, child := range v {
				findProfile(child)
			}
		}
	}
	for _, record := range strings.Split(stream.String(), "\n") {
		colon := strings.IndexByte(record, ':')
		if colon < 1 || colon > 16 {
			continue
		}
		validID := true
		for _, r := range record[:colon] {
			if !((r >= '0' && r <= '9') || (r >= 'a' && r <= 'f')) {
				validID = false
				break
			}
		}
		payload := strings.TrimSpace(record[colon+1:])
		if !validID || payload == "" || (payload[0] != '{' && payload[0] != '[') {
			continue
		}
		decoder := json.NewDecoder(strings.NewReader(payload))
		decoder.UseNumber()
		var value any
		if err := decoder.Decode(&value); err != nil {
			continue
		}
		if err := decoder.Decode(new(any)); !errors.Is(err, io.EOF) {
			continue
		}
		findProfile(value)
	}
	if len(candidates) != 1 {
		return row, errors.New("public profile data unavailable or ambiguous")
	}
	encoded, err := json.Marshal(candidates[0])
	if err != nil {
		return row, err
	}
	var profile profileCursorData
	if err := json.Unmarshal(encoded, &profile); err != nil {
		return row, errors.New("invalid profile metrics")
	}
	if profile.Handle != handle || profile.Visibility != "PUBLIC" || profile.ViewedAs != "external" {
		return row, errors.New("profile is not public or identity changed")
	}
	if len(profile.TokensOverTime) != 30 {
		return row, errors.New("unexpected profile date window")
	}
	var total int64
	var previous time.Time
	for _, point := range profile.TokensOverTime {
		date, err := time.Parse("2006-01-02", point.Date)
		if err != nil || date.After(now.UTC().Add(24*time.Hour)) || (!previous.IsZero() && !date.Equal(previous.AddDate(0, 0, 1))) || point.Tokens == nil || *point.Tokens < 0 || *point.Tokens > profileShareMaxMetric-total {
			return row, errors.New("invalid profile token series")
		}
		total += *point.Tokens
		previous = date
	}
	row = aggregateSource{Provider: "cursor", Status: "live", Tokens: &total, Period: "reported", PeriodStart: profile.TokensOverTime[0].Date, PeriodEnd: profile.TokensOverTime[len(profile.TokensOverTime)-1].Date, PeriodTimezone: "unspecified", Source: "public_ssr", ObservedAt: now.UTC().Format(time.RFC3339)}
	// Agent headline and series differ in the observed source (58 vs 59), and
	// neither establishes a token metric. Do not publish them as API requests.
	return row, nil
}

type profileCursorCacheEntry struct {
	Row     aggregateSource
	Expires time.Time
}

var profileCursorCache = struct {
	sync.Mutex
	Entries map[string]profileCursorCacheEntry
}{Entries: make(map[string]profileCursorCacheEntry)}
var profileCursorSlots = make(chan struct{}, 4)

func fetchProfileCursor(ctx context.Context, rawURL string, now time.Time) aggregateSource {
	row := aggregateSource{Provider: "cursor", URL: rawURL, Status: "unavailable", Period: "unknown", Source: "public_ssr", Reason: "public_metrics_unavailable"}
	err := validateLinkedURL("cursor", rawURL)
	u, _ := url.Parse(rawURL)
	if err != nil {
		row.Reason = "invalid_profile_url"
		return row
	}
	profileCursorCache.Lock()
	cached, ok := profileCursorCache.Entries[rawURL]
	profileCursorCache.Unlock()
	if ok && now.Before(cached.Expires) {
		return cached.Row
	}
	select {
	case profileCursorSlots <- struct{}{}:
		defer func() { <-profileCursorSlots }()
	case <-ctx.Done():
		return row
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return row
	}
	request.Header.Set("Accept", "text/html")
	request.Header.Set("User-Agent", "LMM-ProfileShare/1.0 (public-profile; no-auth)")
	response, err := profileShareHTTPClient.Do(request)
	cacheable := true
	if err == nil {
		defer response.Body.Close()
		control := strings.ToLower(strings.Join(response.Header.Values("Cache-Control"), ","))
		for _, directive := range strings.Split(control, ",") {
			directive = strings.TrimSpace(directive)
			if directive == "no-store" || directive == "no-cache" || strings.HasPrefix(directive, "no-cache=") || directive == "private" || strings.HasPrefix(directive, "private=") || directive == "max-age=0" {
				cacheable = false
			}
		}
		if response.StatusCode == http.StatusOK && strings.HasPrefix(strings.ToLower(response.Header.Get("Content-Type")), "text/html") {
			body, readErr := io.ReadAll(io.LimitReader(response.Body, profileShareCursorMaxBody+1))
			if readErr == nil {
				if parsed, parseErr := parseProfileCursorHTML(body, strings.TrimPrefix(u.Path, "/@"), now); parseErr == nil {
					row = parsed
					row.URL = rawURL
				}
			}
		}
	}
	if !cacheable {
		profileCursorCache.Lock()
		delete(profileCursorCache.Entries, rawURL)
		profileCursorCache.Unlock()
		return row
	}
	// No stale-on-error fallback: private/error/schema-drift responses replace
	// live data. The short positive TTL bounds anonymous public-source staleness.
	ttl := 15 * time.Second
	if row.Status == "live" {
		ttl = 60 * time.Second
	}
	profileCursorCache.Lock()
	if len(profileCursorCache.Entries) >= 128 {
		oldestKey := ""
		var oldest time.Time
		for key, entry := range profileCursorCache.Entries {
			if oldestKey == "" || entry.Expires.Before(oldest) {
				oldestKey, oldest = key, entry.Expires
			}
		}
		delete(profileCursorCache.Entries, oldestKey)
	}
	profileCursorCache.Entries[rawURL] = profileCursorCacheEntry{Row: row, Expires: now.Add(ttl)}
	profileCursorCache.Unlock()
	return row
}

// Reject special-purpose address ranges, including IPv4 mapped IPv6. Validate
// all DNS answers then dial a pinned address, so a second DNS lookup cannot
// turn a previously public hostname into a private destination.
func profileSharePublicIP(ip netip.Addr) bool {
	ip = ip.Unmap()
	if !ip.IsValid() || !ip.IsGlobalUnicast() || ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast() || ip.IsUnspecified() {
		return false
	}
	for _, raw := range []string{"0.0.0.0/8", "100.64.0.0/10", "192.0.0.0/24", "192.0.2.0/24", "198.18.0.0/15", "198.51.100.0/24", "203.0.113.0/24", "240.0.0.0/4", "2001:db8::/32", "2001::/23", "64:ff9b::/96", "64:ff9b:1::/48"} {
		if netip.MustParsePrefix(raw).Contains(ip) {
			return false
		}
	}
	return true
}

func profileShareSafeDial(ctx context.Context, network, address string) (net.Conn, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil || host != "cursor.com" || port != "443" {
		return nil, errors.New("unsupported profile destination")
	}
	ips, err := net.DefaultResolver.LookupNetIP(ctx, "ip", host)
	if err != nil || len(ips) == 0 {
		return nil, errors.New("profile DNS unavailable")
	}
	for _, ip := range ips {
		if !profileSharePublicIP(ip) {
			return nil, errors.New("unsafe profile destination")
		}
	}
	var lastErr error
	for _, ip := range ips {
		conn, err := (&net.Dialer{Timeout: 2 * time.Second}).DialContext(ctx, network, net.JoinHostPort(ip.String(), port))
		if err == nil {
			return conn, nil
		}
		lastErr = err
	}
	return nil, lastErr
}

var profileShareHTTPClient = &http.Client{
	Timeout:       4 * time.Second,
	CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse },
	Transport:     &http.Transport{Proxy: nil, DialContext: profileShareSafeDial, TLSHandshakeTimeout: 2 * time.Second, ResponseHeaderTimeout: 3 * time.Second, MaxConnsPerHost: 2, MaxIdleConns: 4, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, MaxResponseHeaderBytes: 32 << 10},
}
