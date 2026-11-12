// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Package checkins owns daily reward policies and business records in its own DB.
// Only the independently authenticated Core ledger capability can issue credits.
package checkins

import (
	"context"
	"crypto/rand"
	"errors"
	"math"
	"math/big"
	"strconv"
	"time"
)

var (
	ErrInvalid      = errors.New("invalid checkin request")
	ErrUnauthorized = errors.New("checkin authentication required")
	ErrForbidden    = errors.New("checkin forbidden")
	ErrConflict     = errors.New("checkin operation conflict")
	ErrUnavailable  = errors.New("checkin settlement unconfirmed")
)

const Offset = 28800
const SourceRevision = "cc52aa195c6f2a01d703a1a407be73c31b89867d"

type Policy struct {
	Enabled               bool      `json:"enabled"`
	MinQuota              string    `json:"min_quota"`
	MaxQuota              string    `json:"max_quota"`
	LevelMultipliers      []float64 `json:"level_multipliers"`
	CalendarOffsetSeconds int32     `json:"calendar_offset_seconds"`
	CreditsPerUSD         int64     `json:"credits_per_usd"`
	SourceRevision        string    `json:"source_revision"`
	SourceOptionsSHA256   string    `json:"source_options_sha256"`
	HistorySeedID         string    `json:"history_seed_id"`
}
type RewardRange struct {
	TrustLevel       uint32  `json:"trust_level"`
	RewardMultiplier float64 `json:"reward_multiplier"`
	BaseMinQuota     string  `json:"base_min_quota"`
	BaseMaxQuota     string  `json:"base_max_quota"`
	MinQuota         string  `json:"min_quota"`
	MaxQuota         string  `json:"max_quota"`
}
type DailyRecord struct {
	CheckinDate    string  `json:"checkin_date"`
	QuotaAwarded   string  `json:"quota_awarded"`
	CreatedAt      int64   `json:"created_at"`
	JournalID      *int64  `json:"journal_id"`
	Origin         string  `json:"origin"`
	PolicyRevision *int64  `json:"policy_revision"`
	TrustLevel     *uint32 `json:"trust_level"`
}
type ClaimRequest struct {
	OperationKey string `json:"operation_key"`
}
type ClaimResult struct {
	AlreadyClaimed bool        `json:"already_claimed"`
	Record         DailyRecord `json:"record"`
}
type Status struct {
	Enabled               bool          `json:"enabled"`
	Today                 string        `json:"today"`
	Month                 string        `json:"month"`
	CalendarOffsetSeconds int           `json:"calendar_offset_seconds"`
	PolicyRevision        *int64        `json:"policy_revision"`
	CheckedInToday        bool          `json:"checked_in_today"`
	RewardRange           *RewardRange  `json:"reward_range"`
	TodayRecord           *DailyRecord  `json:"today_record"`
	Records               []DailyRecord `json:"records"`
	TotalCheckins         int64         `json:"total_checkins"`
	TotalQuotaAwarded     string        `json:"total_quota_awarded"`
	Pending               bool          `json:"pending"`
}

// Authority comes from fresh Core authentication, never supplied HTTP fields.
type Authority struct {
	UserID, AccountID, Timestamp, MaxDailyUnits int64
	Day                                         string
	TrustLevel                                  uint32
	Blocked                                     bool
}
type Receipt struct {
	UserID, AccountID, AmountUnits, Timestamp int64
	Day                                       string
	JournalID                                 *int64
	Replayed                                  bool
}
type Ledger interface {
	Inspect(context.Context, string) (Authority, error)
	Post(context.Context, string, string, int64, uint32) (Receipt, error)
}

func validKey(key string) bool {
	if len(key) < 16 || len(key) > 80 {
		return false
	}
	for _, b := range []byte(key) {
		if !(b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' || b == '_' || b == '-' || b == ':' || b == '.') {
			return false
		}
	}
	return true
}
func day(timestamp int64) string {
	return time.Unix(timestamp, 0).UTC().Add(Offset * time.Second).Format("2006-01-02")
}
func (p Policy) rewardRange(level uint32) (RewardRange, error) {
	defaults := []float64{0.5, 0.65, 0.8, 0.9, 1}
	index := min(level, 4)
	mult := defaults[index]
	if int(index) < len(p.LevelMultipliers) && p.LevelMultipliers[index] >= 0 && !math.IsNaN(p.LevelMultipliers[index]) && !math.IsInf(p.LevelMultipliers[index], 0) {
		mult = p.LevelMultipliers[index]
	}
	low, e := strconv.ParseInt(p.MinQuota, 10, 64)
	if e != nil || strconv.FormatInt(low, 10) != p.MinQuota {
		return RewardRange{}, ErrInvalid
	}
	high, e := strconv.ParseInt(p.MaxQuota, 10, 64)
	if e != nil || strconv.FormatInt(high, 10) != p.MaxQuota {
		return RewardRange{}, ErrInvalid
	}
	low = max(low, 0)
	high = max(high, low)
	scale := func(v int64) (int64, error) {
		s := math.Round(float64(v) * mult)
		if math.IsNaN(s) || math.IsInf(s, 0) || s < 0 || s >= float64(math.MaxInt64) {
			return 0, ErrInvalid
		}
		return int64(s), nil
	}
	l, e := scale(low)
	if e != nil {
		return RewardRange{}, e
	}
	h, e := scale(high)
	if e != nil {
		return RewardRange{}, e
	}
	return RewardRange{level, mult, p.MinQuota, p.MaxQuota, strconv.FormatInt(l, 10), strconv.FormatInt(max(h, l), 10)}, nil
}
func (p Policy) Validate() error {
	if p.SourceRevision != SourceRevision || p.CalendarOffsetSeconds != Offset || p.CreditsPerUSD != 500000 {
		return ErrInvalid
	}
	for _, m := range p.LevelMultipliers {
		if math.IsNaN(m) || math.IsInf(m, 0) {
			return ErrInvalid
		}
	}
	for l := uint32(0); l < 5; l++ {
		if _, e := p.rewardRange(l); e != nil {
			return e
		}
	}
	return nil
}
func draw(r RewardRange) (int64, error) {
	l, _ := strconv.ParseInt(r.MinQuota, 10, 64)
	h, _ := strconv.ParseInt(r.MaxQuota, 10, 64)
	width := new(big.Int).Sub(big.NewInt(h), big.NewInt(l))
	width.Add(width, big.NewInt(1))
	n, e := rand.Int(rand.Reader, width)
	if e != nil {
		return 0, ErrUnavailable
	}
	return l + n.Int64(), nil
}
