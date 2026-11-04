// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"time"
)

// CoreRead is the client of Core's private, read-only community service on
// the shared private Unix socket, authenticated by its own purpose token.
type CoreRead struct {
	client *http.Client
	token  string
}

var errCoreRead = errors.New("core community read unavailable")

func NewCoreRead(socket string, token []byte) (*CoreRead, error) {
	if !filepath.IsAbs(socket) || filepath.Clean(socket) != socket || strings.ContainsAny(socket, "\x00\r\n") || len(token) < 32 || len(token) > 256 {
		return nil, errInvalid
	}
	for _, b := range token {
		if b < 33 || b > 126 {
			return nil, errInvalid
		}
	}
	dialer := &net.Dialer{Timeout: 2 * time.Second}
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return dialer.DialContext(ctx, "unix", socket)
	}, MaxConnsPerHost: 2, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 10 * time.Second}
	return &CoreRead{client: &http.Client{Transport: transport, Timeout: 12 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, token: string(token)}, nil
}

func (c *CoreRead) Close() {
	if c != nil && c.client != nil {
		c.client.CloseIdleConnections()
	}
}

func (c *CoreRead) call(ctx context.Context, path string, in, out any) error {
	if c == nil || c.client == nil {
		return errCoreRead
	}
	body, err := json.Marshal(in)
	if err != nil {
		return errInvalid
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-community/internal/community/v1/"+path, bytes.NewReader(body))
	if err != nil {
		return errInvalid
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("X-Lmm-Protocol", "1")
	resp, err := c.client.Do(req)
	if err != nil {
		return errCoreRead
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20+1))
	if err != nil || len(raw) > 8<<20 || resp.StatusCode != http.StatusOK {
		return errCoreRead
	}
	if err = json.Unmarshal(raw, out); err != nil {
		return errCoreRead
	}
	return nil
}

// Ready verifies the purpose token and protocol at startup.
func (c *CoreRead) Ready(ctx context.Context) error { return c.ready(ctx, "") }

// ReadyUsage requires the aggregates used by community rankings and profiles.
func (c *CoreRead) ReadyUsage(ctx context.Context) error { return c.ready(ctx, "community.usage.v1") }

// ReadyUsers requires live platform-role fields used by acquisition permissions.
func (c *CoreRead) ReadyUsers(ctx context.Context) error {
	return c.ready(ctx, "community.users.roles.v1")
}

// ReadyNativeCashPayments rejects older Core versions before analytics use.
func (c *CoreRead) ReadyNativeCashPayments(ctx context.Context) error {
	return c.ready(ctx, "community.payments.native.first.v1")
}

func (c *CoreRead) ready(ctx context.Context, required string) error {
	var caps struct {
		Protocol int      `json:"protocol_major"`
		Features []string `json:"features"`
	}
	if err := c.call(ctx, "capabilities", struct{}{}, &caps); err != nil || caps.Protocol != 1 {
		return errCoreRead
	}
	if required != "" {
		for _, feature := range caps.Features {
			if feature == required {
				return nil
			}
		}
		return errCoreRead
	}
	return nil
}

type ModelBucket struct {
	Bucket    int64  `json:"bucket"`
	ModelName string `json:"model_name"`
	Tokens    int64  `json:"tokens"`
	Requests  int64  `json:"requests"`
}

func (c *CoreRead) ModelBuckets(ctx context.Context, start, end, bucket int64) ([]ModelBucket, error) {
	var out []ModelBucket
	err := c.call(ctx, "usage/models", map[string]int64{"start": start, "end": end, "bucket_seconds": bucket}, &out)
	return out, err
}

type CoreRankedUser struct {
	UserID    int64  `json:"user_id"`
	Name      string `json:"name"`
	Anonymous bool   `json:"anonymous"`
	Tokens    int64  `json:"tokens"`
	Requests  int64  `json:"requests"`
}
type CoreUserRankings struct {
	TotalTokens               int64            `json:"total_tokens"`
	TotalRequests             int64            `json:"total_requests"`
	ParticipantCount          int64            `json:"participant_count"`
	AnonymousParticipantCount int64            `json:"anonymous_participant_count"`
	Users                     []CoreRankedUser `json:"users"`
}

func (c *CoreRead) UserRankings(ctx context.Context, start, end int64, limit int) (CoreUserRankings, error) {
	var out CoreUserRankings
	err := c.call(ctx, "usage/users", map[string]int64{"start": start, "end": end, "limit": int64(limit)}, &out)
	return out, err
}

type CoreUsageTotals struct {
	Tokens   int64 `json:"tokens"`
	Requests int64 `json:"requests"`
	Credits  int64 `json:"credits"`
}
type CoreUsageBucket struct {
	Bucket   int64 `json:"bucket"`
	Tokens   int64 `json:"tokens"`
	Requests int64 `json:"requests"`
}
type CoreModelUsage struct {
	ModelName string `json:"model_name"`
	Tokens    int64  `json:"tokens"`
	Requests  int64  `json:"requests"`
	Credits   int64  `json:"credits"`
}
type CoreUserUsage struct {
	UserID        int64             `json:"user_id"`
	Active        bool              `json:"active"`
	Name          string            `json:"name"`
	Username      string            `json:"username"`
	PlatformRole  string            `json:"platform_role"`
	Totals        CoreUsageTotals   `json:"totals"`
	ModelCount    int64             `json:"model_count"`
	Buckets       []CoreUsageBucket `json:"buckets"`
	Models        []CoreModelUsage  `json:"models"`
	CreditsPerUSD int64             `json:"credits_per_usd"`
	HistorySource string            `json:"history_source"`
}

func (c *CoreRead) UserUsage(ctx context.Context, userID, start, end, bucket int64, modelLimit int) (CoreUserUsage, error) {
	var out CoreUserUsage
	err := c.call(ctx, "usage/user", map[string]int64{"user_id": userID, "start": start, "end": end, "bucket_seconds": bucket, "model_limit": int64(modelLimit)}, &out)
	if err == nil && (out.UserID != userID || out.CreditsPerUSD <= 0) {
		err = errCoreRead
	}
	return out, err
}

type CoreUserInfo struct {
	RegisteredAt      *int64 `json:"registered_at"`
	PlatformRole      string `json:"platform_role"`
	UserID            int64  `json:"user_id"`
	Active            bool   `json:"active"`
	Name              string `json:"name"`
	DeveloperAccess   bool   `json:"developer_access"`
	HasActiveKey      bool   `json:"has_active_key"`
	LastAPIActivityAt int64  `json:"last_api_activity_at"`
}

func (c *CoreRead) Users(ctx context.Context, ids []int64) (map[int64]CoreUserInfo, error) {
	var rows []CoreUserInfo
	if err := c.call(ctx, "users", map[string][]int64{"user_ids": ids}, &rows); err != nil {
		return nil, err
	}
	out := make(map[int64]CoreUserInfo, len(rows))
	for _, r := range rows {
		out[r.UserID] = r
	}
	return out, nil
}

type CoreKey struct {
	Active     bool     `json:"active"`
	Group      string   `json:"group"`
	AutoGroups []string `json:"auto_groups"`
}

// Key returns nil when the credential is not one of the user's API keys.
func (c *CoreRead) Key(ctx context.Context, userID, credentialID int64) (*CoreKey, error) {
	var out *CoreKey
	err := c.call(ctx, "keys", map[string]int64{"user_id": userID, "credential_id": credentialID}, &out)
	return out, err
}

// UserPage enumerates current identity without granting Go database access.
type RegistrationUser struct {
	UserID       int64  `json:"user_id"`
	RegisteredAt *int64 `json:"registered_at"`
	PlatformRole string `json:"platform_role"`
	Active       bool   `json:"active"`
}
type RegistrationPage struct {
	Users   []RegistrationUser `json:"users"`
	HasMore bool               `json:"has_more"`
	NextID  int64              `json:"next_id"`
}

func (c *CoreRead) UserPage(ctx context.Context, after int64, limit int) (RegistrationPage, error) {
	var out RegistrationPage
	err := c.call(ctx, "users/page", map[string]int64{"after_id": after, "limit": int64(limit)}, &out)
	return out, err
}

// NativeActivity excludes legacy history; its absence cannot prove no activity.
type ActivityDay struct {
	UserID  int64 `json:"user_id"`
	Day     int64 `json:"day"`
	FirstAt int64 `json:"first_at"`
	LastAt  int64 `json:"last_at"`
}
type NativeActivity struct {
	NativeOnly bool          `json:"native_only"`
	Rows       []ActivityDay `json:"rows"`
}

func (c *CoreRead) NativeActivity(ctx context.Context, ids []int64, from, to int64) (NativeActivity, error) {
	var out NativeActivity
	err := c.call(ctx, "activity/native", struct {
		UserIDs []int64 `json:"user_ids"`
		From    int64   `json:"from"`
		To      int64   `json:"to"`
	}{ids, from, to}, &out)
	return out, err
}

// NativeFirstCashPayments excludes legacy history. A nil timestamp never proves
// that a legacy user has not paid. No payment identifiers or amounts leave Core.
type NativeFirstCashPayment struct {
	UserID      int64  `json:"user_id"`
	FirstPaidAt *int64 `json:"first_paid_at"`
}
type NativeFirstCashPayments struct {
	NativeOnly bool                     `json:"native_only"`
	Users      []NativeFirstCashPayment `json:"users"`
}

func (c *CoreRead) NativeFirstCashPayments(ctx context.Context, ids []int64) (NativeFirstCashPayments, error) {
	var out NativeFirstCashPayments
	err := c.call(ctx, "payments/native/first", struct {
		UserIDs []int64 `json:"user_ids"`
	}{ids}, &out)
	return out, err
}

// FirstCashPayments combines source-bound history and native payment authority.
// Incomplete history carries no purported first-payment timestamp.
type FirstCashPayment struct {
	UserID             int64  `json:"user_id"`
	FirstPaidAt        *int64 `json:"first_paid_at"`
	SecondPaidAt       *int64 `json:"second_paid_at"`
	HistoryComplete    bool   `json:"history_complete"`
	UnclassifiedOrders int64  `json:"unclassified_orders"`
}
type FirstCashPayments struct {
	Users []FirstCashPayment `json:"users"`
}

func (c *CoreRead) ReadyFirstCashPayments(ctx context.Context) error {
	return c.ready(ctx, "community.payments.first.v1")
}

// ReadyRepeatCashPayments requires the additive second-paid timestamp contract.
func (c *CoreRead) ReadyRepeatCashPayments(ctx context.Context) error {
	return c.ready(ctx, "community.payments.repeat.v1")
}
func (c *CoreRead) FirstCashPayments(ctx context.Context, ids []int64) (FirstCashPayments, error) {
	var out FirstCashPayments
	err := c.call(ctx, "payments/first", struct {
		UserIDs []int64 `json:"user_ids"`
	}{ids}, &out)
	return out, err
}

// CashPaymentTotals returns currency amounts for credited orders in [from,to).
// Refunds are cumulative applied refunds of those same orders, not refund-date flows.
type CashCurrency struct {
	Currency     string `json:"currency"`
	PaidMicros   int64  `json:"paid_micros"`
	RefundMicros int64  `json:"refund_micros"`
	NetMicros    int64  `json:"net_micros"`
	Orders       int64  `json:"orders"`
}
type CashPaymentUser struct {
	UserID             int64          `json:"user_id"`
	UnclassifiedOrders int64          `json:"unclassified_orders"`
	Currencies         []CashCurrency `json:"currencies"`
}
type CashPaymentTotals struct {
	Users []CashPaymentUser `json:"users"`
}

func (c *CoreRead) ReadyCashPaymentTotals(ctx context.Context) error {
	return c.ready(ctx, "community.payments.totals.v1")
}
func (c *CoreRead) CashPaymentTotals(ctx context.Context, ids []int64, from, to int64) (CashPaymentTotals, error) {
	var out CashPaymentTotals
	err := c.call(ctx, "payments/totals", struct {
		UserIDs []int64 `json:"user_ids"`
		Start   int64   `json:"start"`
		End     int64   `json:"end"`
	}{ids, from, to}, &out)
	return out, err
}

// ConnectionMilestone carries credential creation, not social-login bindings.
// Incomplete history proves positives only; nil is never a historical negative.
type ConnectionMilestone struct {
	UserID            int64  `json:"user_id"`
	APIKeyCreatedAt   *int64 `json:"api_key_created_at"`
	OAuthAuthorizedAt *int64 `json:"oauth_authorized_at"`
	HistoryComplete   bool   `json:"history_complete"`
}
type ConnectionMilestones struct {
	Users []ConnectionMilestone `json:"users"`
}

func (c *CoreRead) ReadyConnectionMilestones(ctx context.Context) error {
	return c.ready(ctx, "community.connections.v1")
}
func (c *CoreRead) ConnectionMilestones(ctx context.Context, ids []int64) (ConnectionMilestones, error) {
	var out ConnectionMilestones
	err := c.call(ctx, "connections", struct {
		UserIDs []int64 `json:"user_ids"`
	}{ids}, &out)
	return out, err
}
