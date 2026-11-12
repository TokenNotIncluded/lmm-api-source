// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
)

type reviewAuthority struct {
	classified, admitted, approved, invalidated int
	token                                       string
	hook                                        func()
	invalidateErr, classifyErr                  error
	coreActive                                  bool
	expectedLive                                string
}

func (*reviewAuthority) Route(context.Context) (marketreview.Route, error) {
	return marketreview.Route{Group: "synthetic", Model: "review-test", Engine: "openai_moderation", Categories: []string{"violence"}}, nil
}
func (*reviewAuthority) OwnerEnabled(context.Context, int64) (bool, error) { return true, nil }
func (*reviewAuthority) AutoApproveTools() bool                            { return true }
func (a *reviewAuthority) Classify(_ context.Context, r marketreview.Request) (marketreview.Classification, error) {
	a.classified++
	if a.classifyErr != nil {
		return marketreview.Classification{}, a.classifyErr
	}
	if strings.Contains(r.Text, "http") || strings.Contains(r.Text, `"properties"`) {
		return marketreview.Classification{}, errors.New("private endpoint/schema in classification")
	}
	return marketreview.Classification{Model: "review-test", Categories: []string{}, Receipt: "synthetic-trusted-receipt"}, nil
}
func (a *reviewAuthority) AdmitTool(_ context.Context, credential string, r marketreview.Request, quotes []string, live string) (int64, error) {
	if credential != "1" || len(quotes) != 1 || live != a.expectedLive || r.OwnerID != 1 || r.Source != "tool" {
		return 0, ErrDenied
	}
	a.admitted++
	a.token = r.Token
	return 1, nil
}
func (a *reviewAuthority) ApproveTool(_ context.Context, receipt string, quotes []string, token string) (int64, error) {
	if receipt != "synthetic-trusted-receipt" || token != a.token || len(quotes) != 1 {
		return 0, ErrDenied
	}
	a.approved++
	a.coreActive = true
	if a.hook != nil {
		h := a.hook
		a.hook = nil
		h()
	}
	return 2, nil
}
func (a *reviewAuthority) InvalidateTool(_ context.Context, owner int64, token string) error {
	if owner != 1 || token != a.token {
		return ErrDenied
	}
	a.invalidated++
	if a.invalidateErr != nil {
		return a.invalidateErr
	}
	a.coreActive = false
	return nil
}

func TestMarketAIReviewUsesFrozenAdmissionAndSupportsHumanOverride(t *testing.T) {
	m, _, mock := webFixture(t)
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	policy, err := marketreview.NewFilePolicy(filepath.Join(dir, "review.toml"))
	if err != nil {
		t.Fatal(err)
	}
	core := &reviewAuthority{}
	if err = m.ConfigureAIReview(&marketreview.Engine{Policy: policy, Classifier: core}); err != nil {
		t.Fatal(err)
	}
	if err = policy.Save(context.Background(), marketreview.Modes{Tool: marketreview.Auto, Store: marketreview.Off}); err != nil {
		t.Fatal(err)
	}
	create := func() WebServiceRecord {
		s := webSaved(t, m, "1", webInput(mock.server.URL+"/mcp", "public"))
		if _, err := webRequest(t, m, "1", "validate", s.ID, nil); err != nil {
			t.Fatal(err)
		}
		if _, err := webRequest(t, m, "1", "submit", s.ID, map[string]any{"version_id": s.DraftVersionID}); err != nil {
			t.Fatal(err)
		}
		return s
	}
	s := create()
	if err = m.WorkAIReviews(context.Background(), 1); err != nil {
		t.Fatal(err)
	}
	if len(webCatalogue(t, m, "", "")) != 1 || core.admitted != 1 || core.approved != 1 {
		t.Fatal("auto review did not publish through Core admission", core)
	}
	records, err := m.AIReviewHistory(context.Background(), "1", "1", s.ID, "")
	if err != nil || len(records) != 1 || !records[0].Applied {
		t.Fatal(records, err)
	}
	if _, err = m.AIReviewHistory(context.Background(), "2", "2", s.ID, ""); !errors.Is(err, ErrDenied) {
		t.Fatal("private history visible", err)
	}
	// Unrelated owners must not revoke the Core token before ownership checking.
	if _, err = webRequest(t, m, "2", "save", s.ID, webInput(mock.server.URL+"/mcp", "public")); err == nil || core.invalidated != 0 {
		t.Fatal("outsider invalidated another owner's review", err, core)
	}
	if _, err = webRequest(t, m, "3", "review", s.ID, map[string]any{"version_id": s.DraftVersionID, "approve": false, "note": "Manual rejection"}); err != nil {
		t.Fatal(err)
	}
	if core.invalidated != 1 || len(webCatalogue(t, m, "", "")) != 0 {
		t.Fatal("manual override left auto quote enabled")
	}
	records, err = m.AIReviewHistory(context.Background(), "1", "1", s.ID, "")
	if err != nil || *records[0].Outcome != "overridden" {
		t.Fatal(records, err)
	}
	// A successful but late approval ACK cannot override an intervening human.
	delayed := create()
	core.hook = func() {
		if _, err := webRequest(t, m, "3", "review", delayed.ID, map[string]any{"version_id": delayed.DraftVersionID, "approve": false, "note": "Concurrent human rejection"}); err != nil {
			t.Fatal(err)
		}
	}
	if err = m.WorkAIReviews(context.Background(), 1); err != nil {
		t.Fatal(err)
	}
	records, err = m.AIReviewHistory(context.Background(), "1", "1", delayed.ID, "")
	if err != nil || records[0].Applied || *records[0].Outcome != "overridden" {
		t.Fatal("late worker publication", records, err)
	}
	if core.invalidated != 3 || len(webCatalogue(t, m, "", "")) != 0 {
		t.Fatal("late remote approval not revoked", core)
	}
}

// This test reopens the actual encrypted store after the local commit fails.
// Core authority is a deterministic boundary; real Core PostgreSQL approval is
// covered separately by services/core/tests/market_review.rs.
func TestMarketAIReviewRecoversUncommittedCoreApprovalAfterReopen(t *testing.T) {
	for _, scenario := range []struct {
		name        string
		mode        marketreview.Mode
		attempts    int
		classifyErr error
	}{
		{"disabled mode", marketreview.Off, 1, nil},
		{"exhausted attempts", marketreview.Auto, 3, nil},
		{"permanent provider failure", marketreview.Auto, 1, &marketreview.Failure{Code: "provider_invalid"}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			ctx := context.Background()
			dir := t.TempDir()
			if err := os.Chmod(dir, 0700); err != nil {
				t.Fatal(err)
			}
			key := bytes.Repeat([]byte{7}, 32)
			path := filepath.Join(dir, "market.db")
			disk, err := InitStore(path, key)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { _ = disk.Close() })
			mock := newMock(t)
			auth := &webTestAuthority{}
			m, err := New(disk, auth, Config{WebOnly: true, CallsPerMinute: 600})
			if err != nil {
				t.Fatal(err)
			}
			useMock(t, m, mock)
			policy, err := marketreview.NewFilePolicy(filepath.Join(dir, "review.toml"))
			if err != nil {
				t.Fatal(err)
			}
			if err = policy.Save(ctx, marketreview.Modes{Tool: marketreview.Auto, Store: marketreview.Off}); err != nil {
				t.Fatal(err)
			}
			core := &reviewAuthority{invalidateErr: ErrUnavailable}
			engine := &marketreview.Engine{Policy: policy, Classifier: core}
			if err = m.ConfigureAIReview(engine); err != nil {
				t.Fatal(err)
			}
			s := webSaved(t, m, "1", webInput(mock.server.URL+"/mcp", "public"))
			if _, err = webRequest(t, m, "1", "validate", s.ID, nil); err != nil {
				t.Fatal(err)
			}
			if _, err = webRequest(t, m, "1", "submit", s.ID, map[string]any{"version_id": s.DraftVersionID}); err != nil {
				t.Fatal(err)
			}
			core.hook = func() {
				// Simulate losing the local storage/commit after Core has
				// approved. The already persisted dispatch marker survives.
				disk.mu.Lock()
				disk.broken = true
				disk.mu.Unlock()
			}
			if err = m.WorkAIReviews(ctx, 1); !errors.Is(err, ErrUnavailable) || !core.coreActive || core.approved != 1 {
				t.Fatal("did not reach uncertain remote commit", err, core)
			}
			if err = disk.Close(); err != nil {
				t.Fatal(err)
			}
			reopened, err := OpenStore(path, key)
			if err != nil {
				t.Fatal(err)
			}
			defer reopened.Close()
			restarted, err := New(reopened, auth, Config{WebOnly: true, CallsPerMinute: 600})
			if err != nil {
				t.Fatal(err)
			}
			useMock(t, restarted, mock)
			restarted.now = func() time.Time { return time.Now().Add(2 * time.Minute) }
			if err = restarted.ConfigureAIReview(engine); err != nil {
				t.Fatal(err)
			}
			if err = reopened.update(func(d *database) error {
				v := d.Web.Versions[s.DraftVersionID]
				if !v.AIReview.Jobs[0].ApprovalPending || v.Version.Status != "pending" {
					t.Fatal("uncertain approval was not durable")
				}
				v.AIReview.Jobs[0].Attempts = scenario.attempts
				d.Web.Versions[s.DraftVersionID] = v
				return nil
			}); err != nil {
				t.Fatal(err)
			}
			if err = policy.Save(ctx, marketreview.Modes{Tool: scenario.mode, Store: marketreview.Off}); err != nil {
				t.Fatal(err)
			}
			core.invalidateErr, core.classifyErr = nil, scenario.classifyErr
			if err = restarted.WorkAIReviews(ctx, 1); err != nil {
				t.Fatal(err)
			}
			if core.coreActive || core.classified != 1 || core.approved != 1 || core.invalidated != 2 {
				t.Fatal("recovery retried classification/approval or failed revocation", core)
			}
			records, err := restarted.AIReviewHistory(ctx, "1", "1", s.ID, "")
			if err != nil || len(records) != 1 || records[0].Status != "failed" || records[0].Applied || *records[0].Outcome != "manual_required" || *records[0].ErrorCode != "core_approval_uncertain" || records[0].Attempts != scenario.attempts {
				t.Fatal("uncertain approval did not recover to manual review", records, err)
			}
			if err = reopened.view(func(d database) error {
				j := d.Web.Versions[s.DraftVersionID].AIReview.Jobs[0]
				if j.ApprovalPending || j.Text != "" {
					t.Fatal("terminal recovery retained input or pending marker")
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
			if len(webCatalogue(t, restarted, "", "")) != 0 {
				t.Fatal("uncertain publication became public")
			}
		})
	}
}
