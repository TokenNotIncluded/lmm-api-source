// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Package herosms projects Core-owned supplier operations. It holds neither
// supplier credentials nor wallet access, and never retries a paid operation.
package herosms

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

type Failure struct {
	Status int
	Code   string
}

func (f *Failure) Error() string { return f.Code }

var ErrInvalid = &Failure{400, "INVALID_REQUEST"}
var ErrUnavailable = &Failure{503, "UNKNOWN"}

type Core struct {
	client *http.Client
	token  string
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
	}, MaxConnsPerHost: 8, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 21 * time.Second}
	return &Core{client: &http.Client{Transport: transport, Timeout: 22 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, token: string(token)}, nil
}
func (c *Core) Close() {
	if c != nil && c.client != nil {
		c.client.CloseIdleConnections()
	}
}
func (c *Core) Call(ctx context.Context, action, user string, in, out any) error {
	if c == nil || c.client == nil {
		return ErrUnavailable
	}
	raw, err := json.Marshal(in)
	if err != nil || len(raw) > 64<<10 {
		return ErrInvalid
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-hero-sms/internal/hero-sms/v1/"+action, bytes.NewReader(raw))
	if err != nil {
		return ErrInvalid
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("X-LMM-Protocol", "1")
	req.Header.Set("Content-Type", "application/json")
	if user != "" {
		req.Header.Set("X-LMM-User-Credential", user)
	}
	res, err := c.client.Do(req)
	if err != nil {
		return ErrUnavailable
	}
	defer res.Body.Close()
	raw, err = io.ReadAll(io.LimitReader(res.Body, (2<<20)+1))
	if err != nil || len(raw) > 2<<20 {
		return ErrUnavailable
	}
	if res.StatusCode != 200 {
		var f struct {
			Error string `json:"error"`
		}
		_ = json.Unmarshal(raw, &f)
		if f.Error == "" {
			return ErrUnavailable
		}
		switch res.StatusCode {
		case 400, 402, 403, 404, 409, 503:
			return &Failure{res.StatusCode, f.Error}
		default:
			return ErrUnavailable
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	if decoder.Decode(out) != nil || decoder.Decode(new(any)) != io.EOF {
		return ErrUnavailable
	}
	return nil
}
func (c *Core) Ready(ctx context.Context) error {
	var out struct {
		Protocol int    `json:"protocol"`
		Service  string `json:"service"`
		Ready    bool   `json:"ready"`
	}
	if err := c.Call(ctx, "ready", "", struct{}{}, &out); err != nil {
		return err
	}
	if out.Protocol != 1 || out.Service != "hero-sms" || !out.Ready {
		return ErrUnavailable
	}
	return nil
}
func status(err error) (int, string) {
	var f *Failure
	if errors.As(err, &f) {
		return f.Status, f.Code
	}
	return 503, "UNKNOWN"
}

type Selection struct {
	Kind     string `json:"kind"`
	Country  int64  `json:"country"`
	Service  string `json:"service"`
	Operator string `json:"operator"`
	Site     string `json:"site"`
	Domain   string `json:"domain"`
}
type Quote struct {
	ID        string    `json:"id"`
	Selection Selection `json:"selection"`
	Charge    int64     `json:"charge_quota"`
	Price     string    `json:"customer_price_usd"`
	Inventory int64     `json:"inventory"`
	Expires   int64     `json:"expires_at"`
}
type Activation struct {
	ProviderID string `json:"provider_id"`
	Phone      string `json:"phone_number"`
	Email      string `json:"email"`
	Code       string `json:"code"`
	Message    string `json:"message"`
	Status     string `json:"status"`
}
type Receipt struct {
	ID         string      `json:"id"`
	QuoteID    string      `json:"quote_id"`
	State      string      `json:"state"`
	Charge     int64       `json:"charge_quota"`
	Refunded   int64       `json:"refunded_quota"`
	Dispatched bool        `json:"dispatched"`
	ProviderID string      `json:"provider_id"`
	Quote      Quote       `json:"quote"`
	Activation *Activation `json:"activation"`
	Usable     bool        `json:"usable"`
	Created    int64       `json:"created_at"`
	Updated    int64       `json:"updated_at"`
}
type Plan struct {
	ID        string   `json:"id"`
	QuoteID   string   `json:"quote_id"`
	Orders    []string `json:"order_ids"`
	ReorderID string   `json:"reorder_id"`
}
type Page struct {
	Items []Receipt `json:"items"`
	Page  int64     `json:"page"`
	Size  int64     `json:"size"`
	Total int64     `json:"total"`
}
