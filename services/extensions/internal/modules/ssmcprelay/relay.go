// Package ssmcprelay sells monthly ssmcpd relay entitlements through Core Commerce.
package ssmcprelay

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

var (
	ErrRelay         = errors.New("relay accounting unavailable")
	ErrMonth         = errors.New("relay accounting period changed")
	ErrUnbound       = errors.New("relay account is not bound")
	ErrGrantConflict = errors.New("relay entitlement requires reconciliation")
)

type Quota struct {
	UserID           string  `json:"user_id"`
	Username         string  `json:"username"`
	Period           string  `json:"period"`
	UTCOffsetMinutes int     `json:"utc_offset_minutes"`
	ResetAt          int64   `json:"reset_at"`
	FreeBytes        string  `json:"free_bytes"`
	PaidBytes        string  `json:"paid_bytes"`
	UsedBytes        string  `json:"used_bytes"`
	UploadedBytes    string  `json:"uploaded_bytes"`
	DownloadedBytes  string  `json:"downloaded_bytes"`
	LegacyUsedBytes  string  `json:"legacy_used_bytes"`
	LimitBytes       *string `json:"limit_bytes"`
	RemainingBytes   *string `json:"remaining_bytes"`
}

type Entitlement struct {
	UserID  string `json:"user_id"`
	OrderID string `json:"order_id"`
	Period  string `json:"period"`
	Bytes   string `json:"bytes"`
}

type Relay interface {
	Quota(context.Context, int64) (Quota, error)
	Grant(context.Context, Entitlement) error
}
type Client struct {
	base, token string
	http        *http.Client
}

// The private service bearer may travel over plaintext only to a literal loopback host.
func NewClient(baseURL, serviceToken string) (*Client, error) {
	u, err := url.Parse(baseURL)
	if err != nil || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return nil, errors.New("invalid relay accounting origin")
	}
	ip := net.ParseIP(u.Hostname())
	if u.Scheme != "https" && !(u.Scheme == "http" && (u.Hostname() == "localhost" || ip.IsLoopback())) {
		return nil, errors.New("relay accounting origin requires TLS or loopback")
	}
	key, err := hex.DecodeString(serviceToken)
	if err != nil || len(key) != 32 || hex.EncodeToString(key) != serviceToken {
		return nil, errors.New("invalid relay accounting credential")
	}
	return &Client{strings.TrimRight(u.String(), "/"), serviceToken, &http.Client{Timeout: 4 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}}, nil
}

func (c *Client) post(ctx context.Context, path string, in, out any) error {
	body, err := json.Marshal(in)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.base+"/internal/v1/ssmcp/"+path, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("Content-Type", "application/json")
	r, err := c.http.Do(req)
	if err != nil {
		return ErrRelay
	}
	defer r.Body.Close()
	data, err := io.ReadAll(io.LimitReader(r.Body, 16*1024+1))
	if err != nil || len(data) > 16*1024 {
		return ErrRelay
	}
	if r.StatusCode != http.StatusOK {
		var p struct {
			Error string `json:"error"`
		}
		if json.Unmarshal(data, &p) == nil {
			switch {
			case r.StatusCode == http.StatusNotFound && p.Error == "unknown_user":
				return ErrUnbound
			case r.StatusCode == http.StatusConflict && p.Error == "period_mismatch":
				return ErrMonth
			case r.StatusCode == http.StatusConflict && (p.Error == "order_conflict" || p.Error == "ledger_limit"):
				return ErrGrantConflict
			}
		}
		return fmt.Errorf("%w (HTTP %d)", ErrRelay, r.StatusCode)
	}
	if json.Unmarshal(data, out) != nil {
		return ErrRelay
	}
	return nil
}

func validBytes(s string) bool {
	v, e := strconv.ParseUint(s, 10, 64)
	return e == nil && strconv.FormatUint(v, 10) == s
}

func (c *Client) Quota(ctx context.Context, userID int64) (Quota, error) {
	if userID <= 0 {
		return Quota{}, ErrUnbound
	}
	id := strconv.FormatInt(userID, 10)
	var q Quota
	if err := c.post(ctx, "quota", map[string]string{"user_id": id}, &q); err != nil {
		return Quota{}, err
	}
	if _, err := time.Parse("2006-01", q.Period); err != nil || q.UserID != id || q.Username == "" || q.ResetAt <= 0 || q.UTCOffsetMinutes < -720 || q.UTCOffsetMinutes > 840 {
		return Quota{}, ErrRelay
	}
	for _, s := range []string{q.FreeBytes, q.PaidBytes, q.UsedBytes, q.UploadedBytes, q.DownloadedBytes, q.LegacyUsedBytes} {
		if !validBytes(s) {
			return Quota{}, ErrRelay
		}
	}
	if (q.RemainingBytes == nil) != (q.LimitBytes == nil) || (q.RemainingBytes != nil && (!validBytes(*q.RemainingBytes) || !validBytes(*q.LimitBytes))) {
		return Quota{}, ErrRelay
	}
	return q, nil
}

func (c *Client) Grant(ctx context.Context, e Entitlement) error {
	var result struct {
		Entitlement
		Status string `json:"status"`
	}
	if err := c.post(ctx, "entitlements", e, &result); err != nil {
		return err
	}
	if result.Entitlement != e || (result.Status != "granted" && result.Status != "replayed") {
		return ErrRelay
	}
	return nil
}
