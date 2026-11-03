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
func (c *CoreRead) Ready(ctx context.Context) error {
	var caps struct {
		Protocol int      `json:"protocol_major"`
		Features []string `json:"features"`
	}
	if err := c.call(ctx, "capabilities", struct{}{}, &caps); err != nil || caps.Protocol != 1 {
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
