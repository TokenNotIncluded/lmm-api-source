package ssmcpd

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"regexp"
	"strconv"
	"time"
)

var ErrUnbound = errors.New("ssmcpd account not linked")
var ErrMeterUnavailable = errors.New("ssmcpd metering unavailable")
var usernamePattern = regexp.MustCompile("^[a-z][a-z0-9_-]{1,30}[a-z0-9]$")

type Quota struct {
	UserID           string  `json:"user_id"`
	Username         string  `json:"username"`
	Period           string  `json:"period"`
	UTCOffsetMinutes *int    `json:"utc_offset_minutes"`
	ResetAt          *int64  `json:"reset_at"`
	FreeBytes        string  `json:"free_bytes"`
	PaidBytes        string  `json:"paid_bytes"`
	UsedBytes        string  `json:"used_bytes"`
	UploadedBytes    string  `json:"uploaded_bytes"`
	DownloadedBytes  string  `json:"downloaded_bytes"`
	LegacyUsedBytes  string  `json:"legacy_used_bytes"`
	LimitBytes       *string `json:"limit_bytes"`
	RemainingBytes   *string `json:"remaining_bytes"`
}

func (s *Service) MeteringConfigured() bool { return s != nil && s.meter != nil }
func (s *Service) Quota(ctx context.Context, userID int64) (*Quota, error) {
	if userID <= 0 || !s.MeteringConfigured() {
		return nil, ErrMeterUnavailable
	}
	subject := strconv.FormatInt(userID, 10)
	status, body, err := s.meter.post(ctx, "/internal/v1/ssmcp/quota", map[string]string{"user_id": subject})
	if err != nil {
		return nil, ErrMeterUnavailable
	}
	if status == http.StatusNotFound {
		var failure struct {
			Error string `json:"error"`
		}
		if json.Unmarshal(body, &failure) == nil && failure.Error == "unknown_user" {
			return nil, ErrUnbound
		}
		return nil, ErrMeterUnavailable
	}
	if status != http.StatusOK {
		return nil, ErrMeterUnavailable
	}
	var q Quota
	if json.Unmarshal(body, &q) != nil || q.UserID != subject || !usernamePattern.MatchString(q.Username) || q.UTCOffsetMinutes == nil || *q.UTCOffsetMinutes < -14*60 || *q.UTCOffsetMinutes > 14*60 || q.ResetAt == nil || *q.ResetAt <= 0 {
		return nil, ErrMeterUnavailable
	}
	if _, err = time.Parse("2006-01", q.Period); err != nil {
		return nil, ErrMeterUnavailable
	}
	values := []string{q.FreeBytes, q.PaidBytes, q.UsedBytes, q.UploadedBytes, q.DownloadedBytes, q.LegacyUsedBytes}
	if (q.LimitBytes == nil) != (q.RemainingBytes == nil) {
		return nil, ErrMeterUnavailable
	}
	if q.LimitBytes != nil {
		values = append(values, *q.LimitBytes, *q.RemainingBytes)
	} else {
		// The daemon explicitly emits both nulls for unlimited plans. A missing
		// field in a truncated or incompatible response is not an unlimited plan.
		var fields map[string]json.RawMessage
		if json.Unmarshal(body, &fields) != nil || fields["limit_bytes"] == nil || fields["remaining_bytes"] == nil {
			return nil, ErrMeterUnavailable
		}
	}
	for _, v := range values {
		n, err := strconv.ParseUint(v, 10, 64)
		if err != nil || strconv.FormatUint(n, 10) != v {
			return nil, ErrMeterUnavailable
		}
	}
	return &q, nil
}
