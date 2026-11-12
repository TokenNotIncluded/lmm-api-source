// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package marketreview

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func reviewRoute() Route {
	return Route{Group: "synthetic", Model: "moderation-test", Engine: "openai_moderation", Categories: []string{"violence", "sexual"}}
}
func reviewHistory(t *testing.T, mode Mode) *History {
	t.Helper()
	h := &History{}
	if err := h.Enqueue("product", "item", "v1", 1, mode, reviewRoute(), "Synthetic public description", nil, 100); err != nil {
		t.Fatal(err)
	}
	return h
}
func cleanResult() Classification {
	return Classification{Model: "moderation-test", Categories: []string{}, Receipt: "synthetic-Core-receipt"}
}

func TestRouteFreezesNonemptyUniqueCategorySet(t *testing.T) {
	for _, categories := range [][]string{nil, {}, {"violence", "violence"}, {" violence"}, {"violence\n"}} {
		route := reviewRoute()
		route.Categories = categories
		if route.Valid() {
			t.Fatal("invalid category contract accepted", categories)
		}
	}
	if !reviewRoute().Valid() {
		t.Fatal("valid category contract rejected")
	}
}

func TestSubmissionModeTokenAndHumanDecisionStayAuthoritative(t *testing.T) {
	for _, pair := range [][2]Mode{{Assist, Auto}, {Auto, Assist}, {Auto, Off}} {
		h := reviewHistory(t, pair[0])
		j := h.Claim(101)
		apply, _ := h.Complete(*j, cleanResult(), nil, pair[1], true, true, "", 102)
		if apply || h.Jobs[0].Applied || *h.Jobs[0].Outcome != "reference" || h.Jobs[0].Text != "" {
			t.Fatal("mode switch promoted recommendation", h.Jobs[0])
		}
	}
	h := reviewHistory(t, Auto)
	j := h.Claim(101)
	h.Invalidate("overridden", 102)
	if apply, _ := h.Complete(*j, cleanResult(), nil, Auto, true, true, "", 103); apply {
		t.Fatal("late worker overrode manual review")
	}
	if *h.Jobs[0].Outcome != "overridden" || h.Jobs[0].Text != "" {
		t.Fatal("manual history lost")
	}
	if err := h.Enqueue("product", "item", "v2", 1, Auto, reviewRoute(), "New listing", nil, 104); err != nil {
		t.Fatal(err)
	}
	if h.Token == j.Token {
		t.Fatal("resubmission reused token")
	}
	if apply, _ := h.Complete(*j, cleanResult(), nil, Auto, true, true, "", 105); apply {
		t.Fatal("stale attempt changed new submission")
	}
	next := h.Claim(106)
	if apply, approve := h.Complete(*next, cleanResult(), nil, Auto, true, true, "", 107); !apply || !approve {
		t.Fatal("eligible current classification did not apply")
	}
	if apply, _ := h.Complete(*next, cleanResult(), nil, Auto, true, true, "", 108); apply {
		t.Fatal("duplicate completion applied twice")
	}
}

func TestBoundedRecoveryAndTerminalRedaction(t *testing.T) {
	h := reviewHistory(t, Auto)
	for attempt := 1; attempt <= 3; attempt++ {
		now := int64(100 + attempt*100)
		j := h.Claim(now)
		if j == nil || j.Attempts != attempt {
			t.Fatal("claim", attempt, j)
		}
		if h.Claim(now+1) != nil {
			t.Fatal("live lease duplicated provider call")
		}
		h.Complete(*j, Classification{}, &Failure{"provider_unavailable", true}, Auto, true, true, "", now+2)
	}
	if h.Jobs[0].Status != "failed" || *h.Jobs[0].Outcome != "manual_required" || h.Jobs[0].Text != "" || h.Claim(9999) != nil {
		t.Fatal("retry limit or redaction", h.Jobs[0])
	}
	for _, scenario := range []struct {
		name             string
		enabled, current bool
		manual           string
	}{{"disabled", false, true, ""}, {"stale", true, false, ""}, {"technical", true, true, "technical_validation_failed"}} {
		t.Run(scenario.name, func(t *testing.T) {
			h := reviewHistory(t, Auto)
			j := h.Claim(101)
			if apply, _ := h.Complete(*j, cleanResult(), nil, Auto, scenario.current, scenario.enabled, scenario.manual, 102); apply {
				t.Fatal("ineligible listing approved")
			}
			if h.Jobs[0].Text != "" {
				t.Fatal("terminal text retained")
			}
		})
	}
}

func TestApprovalRecoveryLeaseFencesOriginalWorkerWithoutAnotherClassification(t *testing.T) {
	h := reviewHistory(t, Auto)
	old := h.Claim(101)
	h.Jobs[0].ApprovalPending = true
	next := h.Claim(162)
	if next == nil || next.Attempts != old.Attempts || next.LeaseUntil == old.LeaseUntil {
		t.Fatal("recovery did not acquire a distinct lease without a model attempt")
	}
	if applied, _ := h.Complete(*old, cleanResult(), nil, Auto, true, true, "", 163); applied || !h.Jobs[0].ApprovalPending || h.Jobs[0].LeaseUntil != next.LeaseUntil {
		t.Fatal("expired worker overrode the recovery lease")
	}
}

func TestPublicTextAllowlistAndStrictSettings(t *testing.T) {
	text, err := PublicText("Title", "Link https://secret.example/path?key=PRIVATE and mail me@example.com Bearer TOKEN_SECRET sk-abcdefghijklmnopqrstuvwxyz")
	if err != nil || strings.Contains(text, "secret.example") || strings.Contains(text, "me@example.com") || strings.Contains(text, "TOKEN_SECRET") || strings.Contains(text, "sk-") {
		t.Fatal("private prose leaked", text, err)
	}
	if _, err = PublicText(strings.Repeat("a", 16001)); !errors.Is(err, ErrInvalid) {
		t.Fatal("oversized input truncated")
	}
	for _, raw := range []string{`{}`, `{"tool_mode":"off"}`, `{"tool_mode":"auto","store_mode":"unknown"}`, `{"tool_mode":"off","store_mode":"off","extra":1}`, `{"tool_mode":"off","store_mode":"off"} {}`} {
		if _, err := DecodeModes([]byte(raw)); !errors.Is(err, ErrInvalid) {
			t.Fatal("accepted", raw)
		}
	}
	if m, err := DecodeModes([]byte(`{"tool_mode":"assist","store_mode":"auto"}`)); err != nil || m != (Modes{Assist, Auto}) {
		t.Fatal(m, err)
	}
}

func TestSettingsPersistAndSerializeWithCompletion(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "review.toml")
	p, err := NewFilePolicy(path)
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if err = p.With(ctx, func(m Modes) error {
		if m != (Modes{Off, Off}) {
			t.Fatal(m)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if err = p.Save(ctx, Modes{Auto, Auto}); err != nil {
		t.Fatal(err)
	}
	reopened, err := NewFilePolicy(path)
	if err != nil {
		t.Fatal(err)
	}
	if err = p.With(ctx, func(m Modes) error {
		if m != (Modes{Auto, Auto}) {
			t.Fatal("not persisted", m)
		}
		bounded, cancel := context.WithTimeout(ctx, 30*time.Millisecond)
		defer cancel()
		if err := reopened.Save(bounded, Modes{Off, Off}); !errors.Is(err, context.DeadlineExceeded) {
			t.Fatal("settings changed inside completion transaction", err)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if err = reopened.Save(ctx, Modes{Off, Assist}); err != nil {
		t.Fatal(err)
	}
	if err = p.With(ctx, func(m Modes) error {
		if m != (Modes{Off, Assist}) {
			t.Fatal("read stale config", m)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}
