// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

// Port of the legacy service/rankings.go and service/user_rankings.go. Usage
// aggregates come from Core's private community read; this module only shapes
// the public snapshot and caches it for five minutes.

import (
	"context"
	"math"
	"net/http"
	"sort"
	"strings"
	"sync"
	"time"
)

const (
	rankingCacheTTL         = 5 * time.Minute
	rankingLeaderboardLimit = 20
	rankingHistoryLimit     = 10
	rankingVendorLimit      = 5
	rankingMoverLimit       = 6
	rankingOthersLabel      = "Others"
	rankingUnknownVendor    = "Unknown"
)

type RankingsResponse struct {
	Models             []RankedModel      `json:"models"`
	Vendors            []RankedVendor     `json:"vendors"`
	TopMovers          []RankingMover     `json:"top_movers"`
	TopDroppers        []RankingMover     `json:"top_droppers"`
	ModelsHistory      ModelHistorySeries `json:"models_history"`
	VendorShareHistory VendorShareSeries  `json:"vendor_share_history"`
}
type RankedModel struct {
	Rank         int     `json:"rank"`
	PreviousRank *int    `json:"previous_rank,omitempty"`
	ModelName    string  `json:"model_name"`
	Vendor       string  `json:"vendor"`
	Category     string  `json:"category"`
	TotalTokens  int64   `json:"total_tokens"`
	Share        float64 `json:"share"`
	GrowthPct    float64 `json:"growth_pct"`
}
type RankedVendor struct {
	Rank        int     `json:"rank"`
	Vendor      string  `json:"vendor"`
	TotalTokens int64   `json:"total_tokens"`
	Share       float64 `json:"share"`
	GrowthPct   float64 `json:"growth_pct"`
	ModelsCount int     `json:"models_count"`
	TopModel    string  `json:"top_model"`
}
type RankingMover struct {
	ModelName   string  `json:"model_name"`
	Vendor      string  `json:"vendor"`
	RankDelta   int     `json:"rank_delta"`
	CurrentRank int     `json:"current_rank"`
	GrowthPct   float64 `json:"growth_pct"`
}
type ModelHistoryPoint struct {
	Ts     string `json:"ts"`
	Label  string `json:"label"`
	Model  string `json:"model"`
	Vendor string `json:"vendor"`
	Tokens int64  `json:"tokens"`
}
type ModelHistoryModel struct {
	Name   string `json:"name"`
	Vendor string `json:"vendor"`
	Total  int64  `json:"total"`
}
type ModelHistorySeries struct {
	Points  []ModelHistoryPoint `json:"points"`
	Models  []ModelHistoryModel `json:"models"`
	Buckets int                 `json:"buckets"`
}
type VendorSharePoint struct {
	Ts     string  `json:"ts"`
	Label  string  `json:"label"`
	Vendor string  `json:"vendor"`
	Share  float64 `json:"share"`
	Tokens int64   `json:"tokens"`
}
type VendorShareVendor struct {
	Name  string  `json:"name"`
	Total int64   `json:"total"`
	Share float64 `json:"share"`
}
type VendorShareSeries struct {
	Points  []VendorSharePoint  `json:"points"`
	Vendors []VendorShareVendor `json:"vendors"`
	Buckets int                 `json:"buckets"`
}

type UserUsageRankingsResponse struct {
	Period                    string            `json:"period"`
	UpdatedAt                 int64             `json:"updated_at"`
	TotalTokens               int64             `json:"total_tokens"`
	TotalRequests             int64             `json:"total_requests"`
	ParticipantCount          int64             `json:"participant_count"`
	AnonymousParticipantCount int64             `json:"anonymous_participant_count"`
	Users                     []RankedUserUsage `json:"users"`
}
type RankedUserUsage struct {
	Rank        int     `json:"rank"`
	Name        string  `json:"name,omitempty"`
	Anonymous   bool    `json:"anonymous"`
	TotalTokens int64   `json:"total_tokens"`
	Requests    int64   `json:"requests"`
	Share       float64 `json:"share"`
}

type rankingPeriod struct {
	id          string
	duration    time.Duration
	bucketSize  int64
	labelLayout string
}

func rankingConfig(period string) (rankingPeriod, bool) {
	switch period {
	case "", "week":
		return rankingPeriod{"week", 7 * 24 * time.Hour, 86400, "Jan 2"}, true
	case "today":
		return rankingPeriod{"today", 24 * time.Hour, 3600, "15:04"}, true
	case "month":
		return rankingPeriod{"month", 30 * 24 * time.Hour, 86400, "Jan 2"}, true
	case "year":
		return rankingPeriod{"year", 365 * 24 * time.Hour, 7 * 86400, "Jan 2"}, true
	}
	return rankingPeriod{}, false
}

type rankingTotal struct {
	model  string
	tokens int64
}

// Core's per-model aggregates carry no catalogue vendor (vendor names are
// private provider policy there), so the public vendor is inferred from the
// conventional model family prefix.
var vendorPrefixes = []struct{ prefix, vendor string }{
	{"gpt", "OpenAI"}, {"o1", "OpenAI"}, {"o3", "OpenAI"}, {"o4", "OpenAI"}, {"chatgpt", "OpenAI"}, {"dall-e", "OpenAI"}, {"codex", "OpenAI"}, {"text-embedding", "OpenAI"}, {"whisper", "OpenAI"}, {"tts", "OpenAI"},
	{"claude", "Anthropic"}, {"gemini", "Google"}, {"gemma", "Google"}, {"imagen", "Google"}, {"veo", "Google"},
	{"deepseek", "DeepSeek"}, {"grok", "xAI"}, {"qwen", "Alibaba"}, {"qwq", "Alibaba"}, {"glm", "Zhipu AI"}, {"kimi", "Moonshot AI"}, {"moonshot", "Moonshot AI"},
	{"llama", "Meta"}, {"mistral", "Mistral AI"}, {"mixtral", "Mistral AI"}, {"codestral", "Mistral AI"}, {"doubao", "ByteDance"}, {"seed", "ByteDance"}, {"minimax", "MiniMax"}, {"abab", "MiniMax"},
	{"hunyuan", "Tencent"}, {"ernie", "Baidu"}, {"step", "StepFun"}, {"yi-", "01.AI"}, {"command", "Cohere"}, {"sonar", "Perplexity"}, {"flux", "Black Forest Labs"},
}

func modelVendor(name string) string {
	n := strings.ToLower(name)
	if i := strings.LastIndex(n, "/"); i >= 0 {
		n = n[i+1:]
	}
	for _, p := range vendorPrefixes {
		if strings.HasPrefix(n, p.prefix) {
			return p.vendor
		}
	}
	return rankingUnknownVendor
}

func rankingTotals(rows []ModelBucket) []rankingTotal {
	sum := map[string]int64{}
	for _, r := range rows {
		sum[r.ModelName] += r.Tokens
	}
	out := make([]rankingTotal, 0, len(sum))
	for m, t := range sum {
		if t > 0 {
			out = append(out, rankingTotal{m, t})
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].tokens != out[j].tokens {
			return out[i].tokens > out[j].tokens
		}
		return out[i].model < out[j].model
	})
	return out
}

func rankingShare(value, total int64) float64 {
	if total <= 0 || value <= 0 {
		return 0
	}
	return roundRanking(float64(value) / float64(total))
}
func rankingGrowth(current, previous int64) float64 {
	if previous <= 0 {
		if current > 0 {
			return 100
		}
		return 0
	}
	return roundRanking(float64(current-previous) / float64(previous) * 100)
}
func roundRanking(v float64) float64 { return math.Round(v*10000) / 10000 }

// buildRankings reproduces the legacy snapshot from current/previous totals and
// current bucketed rows.
func buildRankings(cfg rankingPeriod, current, previous, buckets []ModelBucket) *RankingsResponse {
	totals, prev := rankingTotals(current), rankingTotals(previous)
	var totalTokens int64
	for _, t := range totals {
		totalTokens += t.tokens
	}
	prevRank, prevTokens := map[string]int{}, map[string]int64{}
	for i, t := range prev {
		prevRank[t.model], prevTokens[t.model] = i+1, t.tokens
	}
	out := &RankingsResponse{Models: []RankedModel{}, Vendors: []RankedVendor{}, TopMovers: []RankingMover{}, TopDroppers: []RankingMover{}}
	for i, t := range totals {
		if i >= rankingLeaderboardLimit {
			break
		}
		row := RankedModel{Rank: i + 1, ModelName: t.model, Vendor: modelVendor(t.model), Category: "all", TotalTokens: t.tokens, Share: rankingShare(t.tokens, totalTokens), GrowthPct: rankingGrowth(t.tokens, prevTokens[t.model])}
		if r, ok := prevRank[t.model]; ok {
			row.PreviousRank = &r
		}
		out.Models = append(out.Models, row)
	}
	type agg struct {
		total, previous, topTokens int64
		models                     int
		top                        string
	}
	vendors := map[string]*agg{}
	get := func(v string) *agg {
		if vendors[v] == nil {
			vendors[v] = &agg{}
		}
		return vendors[v]
	}
	for _, t := range totals {
		a := get(modelVendor(t.model))
		a.total += t.tokens
		a.models++
		if t.tokens > a.topTokens {
			a.top, a.topTokens = t.model, t.tokens
		}
	}
	for _, t := range prev {
		get(modelVendor(t.model)).previous += t.tokens
	}
	for name, a := range vendors {
		if a.total > 0 {
			out.Vendors = append(out.Vendors, RankedVendor{Vendor: name, TotalTokens: a.total, Share: rankingShare(a.total, totalTokens), GrowthPct: rankingGrowth(a.total, a.previous), ModelsCount: a.models, TopModel: a.top})
		}
	}
	sort.Slice(out.Vendors, func(i, j int) bool {
		if out.Vendors[i].TotalTokens == out.Vendors[j].TotalTokens {
			return out.Vendors[i].Vendor < out.Vendors[j].Vendor
		}
		return out.Vendors[i].TotalTokens > out.Vendors[j].TotalTokens
	})
	for i := range out.Vendors {
		out.Vendors[i].Rank = i + 1
	}
	// History series: top models/vendors plus Others.
	topModels, topVendors := map[string]bool{}, map[string]bool{}
	models := []ModelHistoryModel{}
	var otherModels int64
	for i, t := range totals {
		if i < rankingHistoryLimit {
			topModels[t.model] = true
			models = append(models, ModelHistoryModel{t.model, modelVendor(t.model), t.tokens})
		} else {
			otherModels += t.tokens
		}
	}
	if otherModels > 0 {
		models = append(models, ModelHistoryModel{rankingOthersLabel, "Various", otherModels})
	}
	vendorRows := []VendorShareVendor{}
	var otherVendors int64
	for i, v := range out.Vendors {
		if i < rankingVendorLimit {
			topVendors[v.Vendor] = true
			vendorRows = append(vendorRows, VendorShareVendor{v.Vendor, v.TotalTokens, v.Share})
		} else {
			otherVendors += v.TotalTokens
		}
	}
	if otherVendors > 0 {
		vendorRows = append(vendorRows, VendorShareVendor{rankingOthersLabel, otherVendors, rankingShare(otherVendors, totalTokens)})
	}
	type bucketAgg struct {
		total          int64
		models, vendor map[string]int64
	}
	byBucket := map[int64]*bucketAgg{}
	for _, r := range buckets {
		b := byBucket[r.Bucket]
		if b == nil {
			b = &bucketAgg{models: map[string]int64{}, vendor: map[string]int64{}}
			byBucket[r.Bucket] = b
		}
		b.total += r.Tokens
		m := r.ModelName
		if !topModels[m] {
			m = rankingOthersLabel
		}
		b.models[m] += r.Tokens
		v := modelVendor(r.ModelName)
		if !topVendors[v] {
			v = rankingOthersLabel
		}
		b.vendor[v] += r.Tokens
	}
	keys := make([]int64, 0, len(byBucket))
	for k := range byBucket {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool { return keys[i] < keys[j] })
	out.ModelsHistory = ModelHistorySeries{Points: []ModelHistoryPoint{}, Models: models, Buckets: len(keys)}
	out.VendorShareHistory = VendorShareSeries{Points: []VendorSharePoint{}, Vendors: vendorRows, Buckets: len(keys)}
	for _, k := range keys {
		b := byBucket[k]
		ts, label := time.Unix(k, 0).UTC().Format(time.RFC3339), time.Unix(k, 0).Format(cfg.labelLayout)
		for _, m := range models {
			if t := b.models[m.Name]; t > 0 {
				out.ModelsHistory.Points = append(out.ModelsHistory.Points, ModelHistoryPoint{ts, label, m.Name, m.Vendor, t})
			}
		}
		for _, v := range vendorRows {
			if t := b.vendor[v.Name]; t > 0 {
				out.VendorShareHistory.Points = append(out.VendorShareHistory.Points, VendorSharePoint{ts, label, v.Name, rankingShare(t, b.total), t})
			}
		}
	}
	for i, t := range totals {
		r, ok := prevRank[t.model]
		if !ok || r == i+1 {
			continue
		}
		row := RankingMover{ModelName: t.model, Vendor: modelVendor(t.model), RankDelta: r - (i + 1), CurrentRank: i + 1, GrowthPct: rankingGrowth(t.tokens, prevTokens[t.model])}
		if row.RankDelta > 0 {
			out.TopMovers = append(out.TopMovers, row)
		} else {
			out.TopDroppers = append(out.TopDroppers, row)
		}
	}
	sort.SliceStable(out.TopMovers, func(i, j int) bool {
		if out.TopMovers[i].RankDelta == out.TopMovers[j].RankDelta {
			return out.TopMovers[i].GrowthPct > out.TopMovers[j].GrowthPct
		}
		return out.TopMovers[i].RankDelta > out.TopMovers[j].RankDelta
	})
	sort.SliceStable(out.TopDroppers, func(i, j int) bool {
		if out.TopDroppers[i].RankDelta == out.TopDroppers[j].RankDelta {
			return out.TopDroppers[i].GrowthPct < out.TopDroppers[j].GrowthPct
		}
		return out.TopDroppers[i].RankDelta < out.TopDroppers[j].RankDelta
	})
	if len(out.TopMovers) > rankingMoverLimit {
		out.TopMovers = out.TopMovers[:rankingMoverLimit]
	}
	if len(out.TopDroppers) > rankingMoverLimit {
		out.TopDroppers = out.TopDroppers[:rankingMoverLimit]
	}
	return out
}

type rankingCache struct {
	mu    sync.Mutex
	items map[string]rankingCacheItem
}
type rankingCacheItem struct {
	expires time.Time
	value   any
}

func (c *rankingCache) get(key string, now time.Time, build func() (any, error)) (any, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if item, ok := c.items[key]; ok && now.Before(item.expires) {
		return item.value, nil
	}
	v, err := build()
	if err != nil {
		return nil, err
	}
	if c.items == nil {
		c.items = map[string]rankingCacheItem{}
	}
	c.items[key] = rankingCacheItem{now.Add(rankingCacheTTL), v}
	return v, nil
}

var errRankingsUnavailable = &apiError{http.StatusServiceUnavailable, "RANKINGS_UNAVAILABLE", "Rankings are temporarily unavailable"}

func (h *handlers) rankingPeriod(w http.ResponseWriter, r *http.Request) (rankingPeriod, bool) {
	cfg, ok := rankingConfig(r.URL.Query().Get("period"))
	if !ok {
		respond(w, nil, &apiError{http.StatusBadRequest, "RANKINGS_INVALID_PERIOD", "invalid ranking period"})
		return cfg, false
	}
	if h.s.core == nil {
		respond(w, nil, errRankingsUnavailable)
		return cfg, false
	}
	return cfg, true
}

func (h *handlers) rankings(w http.ResponseWriter, r *http.Request) {
	cfg, ok := h.rankingPeriod(w, r)
	if !ok {
		return
	}
	now := h.s.now()
	v, err := h.s.rankingCache.get("models:"+cfg.id, now, func() (any, error) {
		ctx, cancel := context.WithTimeout(r.Context(), 25*time.Second)
		defer cancel()
		end := now.Unix()
		start := now.Add(-cfg.duration).Unix()
		prevEnd := start - 1
		prevStart := time.Unix(start, 0).Add(-cfg.duration).Unix()
		current, err := h.s.core.ModelBuckets(ctx, start, end, 0)
		if err != nil {
			return nil, err
		}
		previous, err := h.s.core.ModelBuckets(ctx, prevStart, prevEnd, 0)
		if err != nil {
			return nil, err
		}
		buckets, err := h.s.core.ModelBuckets(ctx, start, end, cfg.bucketSize)
		if err != nil {
			return nil, err
		}
		return buildRankings(cfg, current, previous, buckets), nil
	})
	if err != nil {
		respond(w, nil, errRankingsUnavailable)
		return
	}
	respond(w, v, nil)
}

func (h *handlers) userRankings(w http.ResponseWriter, r *http.Request) {
	// Names and opt-in visibility belong to Core and can change between requests.
	// Do not retain public identities in the model-statistics cache or downstream caches.
	w.Header().Set("Cache-Control", "no-store")
	cfg, ok := h.rankingPeriod(w, r)
	if !ok {
		return
	}
	now := h.s.now()
	ctx, cancel := context.WithTimeout(r.Context(), 25*time.Second)
	defer cancel()
	core, err := h.s.core.UserRankings(ctx, now.Add(-cfg.duration).Unix(), now.Unix(), rankingLeaderboardLimit)
	if err != nil {
		respond(w, nil, errRankingsUnavailable)
		return
	}
	out := &UserUsageRankingsResponse{Period: cfg.id, UpdatedAt: now.Unix(), TotalTokens: core.TotalTokens, TotalRequests: core.TotalRequests, ParticipantCount: core.ParticipantCount, AnonymousParticipantCount: core.AnonymousParticipantCount, Users: []RankedUserUsage{}}
	for i, u := range core.Users {
		row := RankedUserUsage{Rank: i + 1, Anonymous: u.Anonymous, TotalTokens: u.Tokens, Requests: u.Requests}
		if !u.Anonymous {
			row.Name = u.Name
		}
		if core.TotalTokens > 0 {
			row.Share = float64(u.Tokens) / float64(core.TotalTokens)
		}
		out.Users = append(out.Users, row)
	}
	respond(w, out, nil)
}
