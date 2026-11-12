// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package marketreview

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"strconv"
	"strings"
	"sync/atomic"
	"time"
)

// Core connects only to the configured private Unix socket with its separately
// scoped review credential. It never accepts an upstream URL or API key.
type Core struct {
	client    *http.Client
	token     string
	autoTools atomic.Bool
}

func NewCore(socket string, token []byte) (*Core, error) {
	if !filepath.IsAbs(socket) || filepath.Clean(socket) != socket || strings.ContainsAny(socket, "\x00\r\n") || len(token) < 32 || len(token) > 256 {
		return nil, ErrInvalid
	}
	for _, b := range token {
		if b < 33 || b > 126 {
			return nil, ErrInvalid
		}
	}
	dialer := &net.Dialer{Timeout: 2 * time.Second}
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return dialer.DialContext(ctx, "unix", socket)
	}, MaxConnsPerHost: 2, MaxIdleConnsPerHost: 1, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 18 * time.Second}
	return &Core{client: &http.Client{Transport: transport, Timeout: 20 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, token: string(token)}, nil
}
func (c *Core) Close() {
	if c != nil && c.client != nil {
		c.client.CloseIdleConnections()
	}
}

func (c *Core) request(ctx context.Context, action, user string, in, out any) error {
	if c == nil || c.client == nil {
		return ErrUnavailable
	}
	raw, err := json.Marshal(in)
	if err != nil || len(raw) > 32<<10 {
		return ErrInvalid
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-market-review/internal/market-review/v1/"+action, bytes.NewReader(raw))
	if err != nil {
		return ErrInvalid
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("X-LMM-Protocol", "1")
	if user != "" {
		req.Header.Set("X-LMM-User-Credential", user)
	}
	response, err := c.client.Do(req)
	if err != nil {
		return &Failure{"provider_unavailable", true}
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		switch response.StatusCode {
		case 429:
			return &Failure{"provider_rate_limited", true}
		case 400:
			return ErrInvalid
		case 401, 403:
			return ErrForbidden
		case 502, 503, 504:
			return &Failure{"provider_unavailable", true}
		default:
			return ErrUnavailable
		}
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, (64<<10)+1))
	if err != nil || len(body) > 64<<10 {
		return &Failure{"provider_invalid", false}
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.DisallowUnknownFields()
	if decoder.Decode(out) != nil || decoder.Decode(new(any)) != io.EOF {
		return &Failure{"provider_invalid", false}
	}
	return nil
}

type coreRoute struct {
	Group      string   `json:"group"`
	Model      string   `json:"model"`
	Engine     string   `json:"engine"`
	Categories []string `json:"categories"`
}

func (c *Core) Route(ctx context.Context) (Route, error) {
	var out struct {
		ProtocolMajor    int       `json:"protocol_major"`
		Route            coreRoute `json:"route"`
		AutoApproveTools *bool     `json:"auto_approve_tools"`
	}
	if err := c.request(ctx, "capabilities", "", struct{}{}, &out); err != nil {
		return Route{}, err
	}
	r := Route{Group: out.Route.Group, Model: out.Route.Model, Engine: out.Route.Engine, SupportedInputs: []string{"text"}, Categories: out.Route.Categories}
	if out.ProtocolMajor != 1 || !r.Valid() || out.AutoApproveTools == nil {
		return Route{}, ErrUnavailable
	}
	c.autoTools.Store(*out.AutoApproveTools)
	return r, nil
}

func (c *Core) AutoApproveTools() bool { return c != nil && c.autoTools.Load() }

type coreSubmission struct {
	Source      string    `json:"source"`
	TargetID    string    `json:"target_id"`
	Version     string    `json:"version"`
	Token       string    `json:"token"`
	OwnerID     string    `json:"owner_id"`
	ContentHash string    `json:"content_hash"`
	Route       coreRoute `json:"route"`
}

func submission(r Request) coreSubmission {
	source := "store_product"
	if r.Source == "tool" {
		source = "tool_market"
	}
	return coreSubmission{source, r.TargetID, r.Version, r.Token, strconv.FormatInt(r.OwnerID, 10), r.ContentHash, coreRoute{r.Route.Group, r.Route.Model, r.Route.Engine, r.Route.Categories}}
}
func (c *Core) AdmitTool(ctx context.Context, credential string, r Request, quotes []string, expected string) (int64, error) {
	if r.Source != "tool" || credential == "" || !c.AutoApproveTools() {
		return 0, ErrForbidden
	}
	var out struct {
		Token    string `json:"token"`
		Revision int64  `json:"revision"`
	}
	in := struct {
		Submission              coreSubmission `json:"submission"`
		QuoteIDs                []string       `json:"quote_ids"`
		ExpectedActiveVersionID string         `json:"expected_active_version_id"`
	}{submission(r), quotes, expected}
	if err := c.request(ctx, "admit", credential, in, &out); err != nil {
		return 0, err
	}
	if out.Token != r.Token || out.Revision < 1 {
		return 0, ErrUnavailable
	}
	return out.Revision, nil
}
func (c *Core) ApproveTool(ctx context.Context, receipt string, quotes []string, token string) (int64, error) {
	var out struct {
		Token    string `json:"token"`
		Revision int64  `json:"revision"`
		Approved *bool  `json:"approved"`
	}
	if err := c.request(ctx, "approve", "", struct {
		Receipt  string   `json:"receipt"`
		QuoteIDs []string `json:"quote_ids"`
	}{receipt, quotes}, &out); err != nil {
		return 0, err
	}
	if out.Token != token || out.Revision < 1 || out.Approved == nil || !*out.Approved {
		return 0, ErrUnavailable
	}
	return out.Revision, nil
}
func (c *Core) InvalidateTool(ctx context.Context, owner int64, token string) error {
	var out struct {
		Token       string `json:"token"`
		Revision    int64  `json:"revision"`
		Invalidated *bool  `json:"invalidated"`
	}
	if err := c.request(ctx, "invalidate", "", struct {
		OwnerID string `json:"owner_id"`
		Token   string `json:"token"`
	}{strconv.FormatInt(owner, 10), token}, &out); err != nil {
		return err
	}
	if out.Token != token || out.Revision < 0 || out.Invalidated == nil || !*out.Invalidated {
		return ErrUnavailable
	}
	return nil
}
func (c *Core) OwnerEnabled(ctx context.Context, id int64) (bool, error) {
	if id <= 0 {
		return false, ErrInvalid
	}
	var out struct {
		Enabled *bool `json:"enabled"`
	}
	err := c.request(ctx, "owner", "", struct {
		OwnerID string `json:"owner_id"`
	}{strconv.FormatInt(id, 10)}, &out)
	if err != nil {
		return false, err
	}
	if out.Enabled == nil {
		return false, ErrUnavailable
	}
	return *out.Enabled, nil
}
func (c *Core) Authorize(ctx context.Context, credential string, root bool) (int64, error) {
	if credential == "" {
		return 0, ErrInvalid
	}
	var out struct {
		UserID string `json:"user_id"`
	}
	if err := c.request(ctx, "authorize", credential, struct {
		Root bool `json:"root"`
	}{root}, &out); err != nil {
		return 0, err
	}
	id, err := strconv.ParseInt(out.UserID, 10, 64)
	if err != nil || id <= 0 || strconv.FormatInt(id, 10) != out.UserID {
		return 0, ErrUnavailable
	}
	return id, nil
}
func (c *Core) Classify(ctx context.Context, r Request) (Classification, error) {
	source := ""
	if r.Source == "tool" {
		source = "tool_market"
	}
	if r.Source == "product" {
		source = "store_product"
	}
	if source == "" || r.OwnerID <= 0 || !r.Route.Valid() || len(r.Text) > 16000 {
		return Classification{}, ErrInvalid
	}
	in := struct {
		Source   string    `json:"source"`
		TargetID string    `json:"target_id"`
		Version  string    `json:"version"`
		Token    string    `json:"token"`
		OwnerID  string    `json:"owner_id"`
		Hash     string    `json:"content_hash"`
		Text     string    `json:"text"`
		Route    coreRoute `json:"route"`
	}{source, r.TargetID, r.Version, r.Token, strconv.FormatInt(r.OwnerID, 10), r.ContentHash, r.Text, coreRoute{r.Route.Group, r.Route.Model, r.Route.Engine, r.Route.Categories}}
	var out struct {
		Flagged    *bool            `json:"flagged"`
		Categories map[string]*bool `json:"categories"`
		Model      string           `json:"response_model"`
		Receipt    string           `json:"receipt"`
	}
	if err := c.request(ctx, "classify", "", in, &out); err != nil {
		return Classification{}, err
	}
	if out.Flagged == nil || len(out.Categories) != len(r.Route.Categories) || out.Model == "" || out.Receipt == "" {
		return Classification{}, &Failure{"provider_invalid", false}
	}
	result := Classification{Flagged: *out.Flagged, Categories: []string{}, Model: out.Model, Receipt: out.Receipt}
	allowed := map[string]bool{}
	for _, category := range r.Route.Categories {
		allowed[category] = true
	}
	for category, flagged := range out.Categories {
		if !allowed[category] || flagged == nil || !*out.Flagged && *flagged {
			return Classification{}, &Failure{"provider_invalid", false}
		}
		if *flagged {
			result.Categories = append(result.Categories, category)
		}
	}
	return result, nil
}
