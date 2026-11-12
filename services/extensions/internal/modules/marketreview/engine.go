// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Package marketreview owns private listing review records. It has no wallet,
// account sanction, model credential, or ordinary model execution authority.
package marketreview

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"regexp"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"
)

var (
	ErrInvalid     = errors.New("market_review_invalid")
	ErrUnavailable = errors.New("market_review_unavailable")
	ErrForbidden   = errors.New("market_review_forbidden")
)

type Mode string

const (
	Off    Mode = "off"
	Assist Mode = "assist"
	Auto   Mode = "auto"
)

type Modes struct {
	Tool  Mode `json:"tool_mode"`
	Store Mode `json:"store_mode"`
}

func (m Modes) Valid() bool { return validMode(m.Tool) && validMode(m.Store) }
func validMode(m Mode) bool { return m == Off || m == Assist || m == Auto }
func (m Modes) For(source string) Mode {
	if source == "tool" {
		return m.Tool
	}
	if source == "product" {
		return m.Store
	}
	return Off
}

// Policy holds a shared lock through fn. Setting changes take its exclusive
// lock, so switching away from auto cannot race a terminal module transaction.
type Policy interface {
	With(context.Context, func(Modes) error) error
	Save(context.Context, Modes) error
}

type Route struct {
	Group           string   `json:"review_group"`
	Model           string   `json:"review_model"`
	Engine          string   `json:"engine"`
	SupportedInputs []string `json:"supported_inputs"`
	Categories      []string `json:"categories"`
}

func (r Route) Valid() bool {
	label := func(value string, max int) bool {
		return value != "" && len(value) <= max && strings.TrimSpace(value) == value && strings.IndexFunc(value, unicode.IsControl) < 0
	}
	if r.Engine != "openai_moderation" || !label(r.Group, 128) || !label(r.Model, 256) || len(r.Categories) == 0 || len(r.Categories) > 64 {
		return false
	}
	seen := make(map[string]bool, len(r.Categories))
	for _, category := range r.Categories {
		if !label(category, 128) || seen[category] {
			return false
		}
		seen[category] = true
	}
	return true
}

type Request struct {
	Source      string `json:"source"`
	TargetID    string `json:"target_id"`
	Version     string `json:"content_version"`
	Token       string `json:"token"`
	OwnerID     int64  `json:"owner_id"`
	ContentHash string `json:"content_hash"`
	Text        string `json:"text"`
	Route       Route  `json:"route"`
}

type Classification struct {
	Flagged    bool     `json:"flagged"`
	Categories []string `json:"categories"`
	Model      string   `json:"response_model"`
	// Receipt is an opaque Core-issued classification receipt, never a client
	// claim that grants permission to publish a Core-controlled tool quote.
	Receipt string `json:"receipt"`
}

type Classifier interface {
	Route(context.Context) (Route, error)
	Classify(context.Context, Request) (Classification, error)
	OwnerEnabled(context.Context, int64) (bool, error)
}

// ToolAuthority grants only the exact submission's already registered quotes.
// It is separate from classification and never retains a user credential.
type ToolAuthority interface {
	AutoApproveTools() bool
	AdmitTool(context.Context, string, Request, []string, string) (int64, error)
	ApproveTool(context.Context, string, []string, string) (int64, error)
	InvalidateTool(context.Context, int64, string) error
}

// Failure codes are stable public codes. Provider bodies are never retained.
type Failure struct {
	Code      string
	Temporary bool
}

func (e *Failure) Error() string { return e.Code }

type Record struct {
	ID             int64    `json:"id"`
	Source         string   `json:"source"`
	TargetID       string   `json:"target_id"`
	Version        string   `json:"content_version"`
	ContentHash    string   `json:"content_hash"`
	Mode           Mode     `json:"mode"`
	Status         string   `json:"status"`
	Attempts       int      `json:"attempts"`
	ReviewModel    string   `json:"review_model"`
	ResponseModel  string   `json:"response_model"`
	Flagged        *bool    `json:"flagged"`
	Categories     []string `json:"categories"`
	Recommendation *string  `json:"recommendation"`
	Outcome        *string  `json:"outcome"`
	Applied        bool     `json:"applied"`
	ErrorCode      *string  `json:"error_code"`
	CreatedAt      int64    `json:"created_at"`
	UpdatedAt      int64    `json:"updated_at"`
	CompletedAt    int64    `json:"completed_at"`
	CheckedAt      int64    `json:"checked_at"`
	Coverage       string   `json:"coverage"`
}

type Job struct {
	Record
	Token                string `json:"token"`
	OwnerID              int64  `json:"owner_id"`
	Text                 string `json:"text,omitempty"`
	Route                Route  `json:"route"`
	LeaseUntil           int64  `json:"lease_until,omitempty"`
	RetryAt              int64  `json:"retry_at,omitempty"`
	AdmissionAttempted   bool   `json:"admission_attempted,omitempty"`
	AdmissionRevision    int64  `json:"admission_revision,omitempty"`
	ApprovalPending      bool   `json:"approval_pending,omitempty"`
	ApprovalPriorVersion string `json:"approval_prior_version,omitempty"`
}

// History is stored inside the product/version record so submission, token
// replacement and enqueue are the same durable module transaction.
type History struct {
	Sequence int64  `json:"sequence"`
	Token    string `json:"token"`
	Jobs     []Job  `json:"jobs"`
}

type Engine struct {
	Policy     Policy
	Classifier Classifier
}

func (e *Engine) With(ctx context.Context, fn func(Modes) error) error {
	if e == nil || e.Policy == nil {
		return fn(Modes{Off, Off})
	}
	return e.Policy.With(ctx, fn)
}

// PublicText accepts only the caller's explicit allowlist. URLs, email
// addresses and common bearer/API key forms inside prose are removed too.
var privateText = regexp.MustCompile(`(?i)https?://[^\s<>]+|[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}|\b(?:bearer\s+[a-z0-9._~+/=\-]+|sk[-_][a-z0-9_\-]{8,}|lmms_[a-z0-9_\-]+)`)

func PublicText(parts ...string) (string, error) {
	text := strings.Join(parts, "\n")
	if !utf8.ValidString(text) || len(text) > 16000 || strings.ContainsRune(text, 0) {
		return "", ErrInvalid
	}
	text = privateText.ReplaceAllString(text, "[redacted]")
	if strings.TrimSpace(text) == "" {
		return "", ErrInvalid
	}
	return text, nil
}

func (h *History) Enqueue(source, target, version string, owner int64, mode Mode, route Route, text string, inputErr error, now int64) error {
	h.Invalidate("stale", now)
	if mode == Off {
		return nil
	}
	if !validMode(mode) || owner <= 0 || target == "" || version == "" || source != "tool" && source != "product" || h.Sequence == 1<<63-1 {
		return ErrInvalid
	}
	var token [16]byte
	if _, err := rand.Read(token[:]); err != nil {
		return err
	}
	h.Token = hex.EncodeToString(token[:])
	h.Sequence++
	sum := sha256.Sum256([]byte(text))
	j := Job{Record: Record{ID: h.Sequence, Source: source, TargetID: target, Version: version, ContentHash: hex.EncodeToString(sum[:]), Mode: mode, Status: "pending", ReviewModel: route.Model, CreatedAt: now, UpdatedAt: now, Coverage: "public_listing_text"}, Token: h.Token, OwnerID: owner, Text: text, Route: route}
	if inputErr != nil {
		j.fail("input_limit", now)
	} else if !route.Valid() {
		j.fail("route_unavailable", now)
	}
	h.Jobs = append(h.Jobs, j)
	if len(h.Jobs) > 20 {
		h.Jobs = append([]Job(nil), h.Jobs[len(h.Jobs)-20:]...)
	}
	return nil
}

func ptr[T any](v T) *T { return &v }
func (j *Job) terminal(status, outcome string, now int64) {
	j.Status, j.Outcome, j.UpdatedAt, j.CompletedAt = status, ptr(outcome), now, now
	j.Text, j.LeaseUntil, j.RetryAt, j.ApprovalPending = "", 0, 0, false
}
func (j *Job) fail(code string, now int64) {
	j.ErrorCode = ptr(code)
	j.terminal("failed", "manual_required", now)
}

func (h *History) Invalidate(outcome string, now int64) {
	h.Token = ""
	for i := range h.Jobs {
		j := &h.Jobs[i]
		if j.Status == "pending" || j.Status == "running" {
			j.terminal("cancelled", outcome, now)
		} else if j.Applied {
			j.Applied = false
			j.Outcome, j.UpdatedAt = ptr(outcome), now
		}
	}
}

func (h *History) Automatic() bool {
	for _, j := range h.Jobs {
		if j.Token == h.Token && j.Applied {
			return true
		}
	}
	return false
}

func (h *History) Manual(code string, now int64) {
	for i := range h.Jobs {
		if h.Jobs[i].Token == h.Token && (h.Jobs[i].Status == "pending" || h.Jobs[i].Status == "running") {
			h.Jobs[i].fail(code, now)
		}
	}
}

func (h *History) Claim(now int64) *Job {
	for i := range h.Jobs {
		j := &h.Jobs[i]
		if j.Token != h.Token || j.RetryAt > now || j.LeaseUntil > now {
			continue
		}
		if !j.ApprovalPending && j.Status != "pending" && j.Status != "running" {
			continue
		}
		// A Core approval sent before a process crash must be reconciled even
		// after the classifier's retry budget is exhausted. Recovery itself
		// neither calls the classifier nor consumes another classification try.
		if j.Attempts >= 3 && !j.ApprovalPending {
			j.fail("attempts_exhausted", now)
			continue
		}
		if !j.ApprovalPending {
			j.Attempts++
		}
		j.Status, j.UpdatedAt, j.LeaseUntil = "running", now, now+60
		copy := *j
		return &copy
	}
	return nil
}

func (j Job) Request() Request {
	return Request{j.Source, j.TargetID, j.Version, j.Token, j.OwnerID, j.ContentHash, j.Text, j.Route}
}

func (e *Engine) Classify(ctx context.Context, j Job) (Classification, error) {
	if e == nil || e.Classifier == nil {
		return Classification{}, &Failure{"route_unavailable", false}
	}
	if err := e.With(ctx, func(m Modes) error {
		if m.For(j.Source) == Off {
			return &Failure{"review_disabled", false}
		}
		return nil
	}); err != nil {
		return Classification{}, err
	}
	on, err := e.Classifier.OwnerEnabled(ctx, j.OwnerID)
	if err != nil {
		return Classification{}, &Failure{"authority_unavailable", true}
	}
	if !on {
		return Classification{}, &Failure{"owner_disabled", false}
	}
	return e.Classifier.Classify(ctx, j.Request())
}

// Complete is called under Policy.With and the same transaction that changes
// the listing. Only the exact outstanding attempt can publish. Technical/Core
// approval failure is represented by manualCode; classification remains useful.
func (h *History) Complete(claim Job, result Classification, callErr error, mode Mode, current, ownerEnabled bool, manualCode string, now int64) (apply, approve bool) {
	for i := range h.Jobs {
		j := &h.Jobs[i]
		if j.ID != claim.ID || j.Token != claim.Token || j.Status != "running" || j.Attempts != claim.Attempts {
			continue
		}
		if h.Token != j.Token || !current {
			j.terminal("cancelled", "stale", now)
			return
		}
		if !ownerEnabled {
			j.fail("owner_disabled", now)
			return
		}
		if callErr != nil {
			code, temporary := "classification_failed", false
			var f *Failure
			if errors.As(callErr, &f) {
				switch f.Code {
				case "review_disabled", "route_unavailable", "provider_unavailable", "provider_rate_limited", "provider_invalid", "authority_unavailable", "owner_disabled":
					code, temporary = f.Code, f.Temporary
				}
			}
			if temporary && j.Attempts < 3 {
				j.Status, j.RetryAt, j.LeaseUntil, j.UpdatedAt = "pending", now+int64(j.Attempts*5), 0, now
				j.ErrorCode = ptr(code)
				return
			}
			j.fail(code, now)
			return
		}
		if result.Model == "" || len(result.Model) > 256 || len(result.Categories) > 64 || result.Receipt == "" {
			j.fail("provider_invalid", now)
			return
		}
		allowed := map[string]bool{}
		for _, category := range j.Route.Categories {
			allowed[category] = true
		}
		for _, category := range result.Categories {
			if !allowed[category] {
				j.fail("provider_invalid", now)
				return
			}
		}
		j.ResponseModel, j.Flagged, j.Categories, j.CheckedAt = result.Model, ptr(result.Flagged), append([]string{}, result.Categories...), now
		sort.Strings(j.Categories)
		approve = !result.Flagged
		recommendation := "reject"
		if approve {
			recommendation = "approve"
		}
		j.Recommendation, j.ErrorCode = ptr(recommendation), nil
		if j.Mode != Auto || mode != Auto {
			j.terminal("completed", "reference", now)
			return false, approve
		}
		if manualCode != "" {
			j.ErrorCode = ptr(manualCode)
			j.terminal("completed", "manual_required", now)
			return false, approve
		}
		j.Applied = true
		outcome := "rejected"
		if approve {
			outcome = "approved"
		}
		j.terminal("completed", outcome, now)
		return true, approve
	}
	return
}

func (h *History) Records() []Record {
	rows := make([]Record, 0, len(h.Jobs))
	for i := len(h.Jobs) - 1; i >= 0; i-- {
		rows = append(rows, h.Jobs[i].Record)
	}
	return rows
}

func DecodeModes(raw []byte) (Modes, error) {
	var m struct {
		Tool  *Mode `json:"tool_mode"`
		Store *Mode `json:"store_mode"`
	}
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&m) != nil || m.Tool == nil || m.Store == nil {
		return Modes{}, ErrInvalid
	}
	var extra any
	if decoder.Decode(&extra) != io.EOF {
		return Modes{}, ErrInvalid
	}
	out := Modes{*m.Tool, *m.Store}
	if !out.Valid() {
		return Modes{}, ErrInvalid
	}
	return out, nil
}
