// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Real PostgreSQL/module workflow; the classifier and identity authority are
// explicit synthetic boundaries, not proof of production Core connectivity.
package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type reviewClassifier struct {
	calls   int
	hook    func()
	fail    bool
	flagged bool
}

func (*reviewClassifier) Route(context.Context) (marketreview.Route, error) {
	return marketreview.Route{Group: "synthetic", Model: "test-moderation", Engine: "openai_moderation", Categories: []string{"violence"}}, nil
}
func (*reviewClassifier) OwnerEnabled(context.Context, int64) (bool, error) { return true, nil }
func (c *reviewClassifier) Classify(_ context.Context, r marketreview.Request) (marketreview.Classification, error) {
	c.calls++
	if strings.Contains(r.Text, "PRIVATE_") || strings.Contains(r.Text, "secret.example") || strings.Contains(r.Text, "me@example.com") {
		return marketreview.Classification{}, errors.New("private field reached classifier")
	}
	if c.hook != nil {
		f := c.hook
		c.hook = nil
		f()
	}
	if c.fail {
		return marketreview.Classification{}, &marketreview.Failure{Code: "provider_unavailable", Temporary: true}
	}
	categories := []string{}
	if c.flagged {
		categories = []string{"violence"}
	}
	return marketreview.Classification{Flagged: c.flagged, Model: "test-moderation", Categories: categories, Receipt: "synthetic-core-classification"}, nil
}

func TestPostgresMarketAIReviewSubmissionRecoveryAndOverride(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	ledger := &funds{seen: map[string]store.Receipt{}}
	s, err := store.New(repo, legacyAuthority{}, ledger)
	must(t, err)
	dir := t.TempDir()
	must(t, os.Chmod(dir, 0700))
	policy, err := marketreview.NewFilePolicy(filepath.Join(dir, "review.toml"))
	must(t, err)
	classifier := &reviewClassifier{}
	engine := &marketreview.Engine{Policy: policy, Classifier: classifier}
	must(t, s.ConfigureAIReview(engine))
	ctx := context.Background()
	actor := store.Account{Kind: "personal", ID: 1}
	admin := store.Account{Kind: "personal", ID: 4}
	body := func(value any) json.RawMessage { b, e := json.Marshal(value); must(t, e); return b }
	invoke := func(token string, who store.Account, r store.LegacyRequest) any {
		t.Helper()
		out, e := s.Legacy(ctx, token, who, r)
		must(t, e)
		return out
	}
	invoke("seller", actor, store.LegacyRequest{Action: "save_terms", Body: body(map[string]any{"content": "Synthetic terms", "expected_version": ""})})
	input := map[string]any{"title": "Synthetic listing", "description": "Public description at https://secret.example/?token=PRIVATE_URL me@example.com", "image_urls": []string{"https://images.example/PRIVATE_IMAGE"}, "contact": "PRIVATE_CONTACT", "links": []any{map[string]string{"title": "Public link title", "url": "https://secret.example/PRIVATE_LINK", "description": "PRIVATE_LINK_DESCRIPTION"}}, "price_quota": int64(500000), "template": "fixed-content", "fixed_content": "PRIVATE_STOCK_SECRET", "delivery_strategy": "sequential", "payment_methods": []string{"balance"}, "pickup_login_required": true, "pickup_code_required": false, "email_pickup_link": false}
	makeProduct := func() string {
		out := invoke("seller", actor, store.LegacyRequest{Action: "save_product", Body: body(input)})
		return out.(map[string]any)["id"].(string)
	}
	submit := func(id string) { invoke("seller", actor, store.LegacyRequest{Action: "submit_product", ID: id}) }
	history := func(id string) []marketreview.Record {
		out, e := s.AIReviewHistory(ctx, "seller", actor, id)
		must(t, e)
		return out
	}
	must(t, policy.Save(ctx, marketreview.Modes{Tool: marketreview.Off, Store: marketreview.Auto}))
	id := makeProduct()
	submit(id)
	rows, err := repo.PendingAIReviews(ctx, 1<<62, 20)
	must(t, err)
	if len(rows) != 1 || rows[0].Product.Legacy.Status != "pending" || len(rows[0].Product.AIReview.Jobs) != 1 {
		t.Fatal("submission and queue not committed together")
	}
	// Rebuild the module before processing: the job is durable, not a goroutine.
	restarted, err := store.New(repo, legacyAuthority{}, ledger)
	must(t, err)
	must(t, restarted.ConfigureAIReview(engine))
	must(t, restarted.WorkAIReviews(ctx, 1))
	public, err := restarted.PublicProduct(ctx, id)
	must(t, err)
	raw := string(body(public))
	if strings.Contains(raw, "ai_review") || strings.Contains(raw, "receipt") || strings.Contains(raw, "PRIVATE_STOCK_SECRET") {
		t.Fatal("private review or delivery exposed")
	}
	if records := history(id); len(records) != 1 || !records[0].Applied || *records[0].Outcome != "approved" {
		t.Fatal(records)
	}
	var persisted string
	must(t, db.QueryRow(`SELECT COALESCE(body->'ai_review'->'jobs'->0->>'text','') FROM lmm_store.products WHERE id=$1`, id).Scan(&persisted))
	if persisted != "" {
		t.Fatal("terminal text persisted")
	}
	queueView := invoke("admin", admin, store.LegacyRequest{Action: "reviews", Limit: 20}).(map[string]any)
	if len(queueView["items"].([]map[string]any)) != 1 {
		t.Fatal("automatic result missing from manual queue")
	}
	invoke("admin", admin, store.LegacyRequest{Action: "review_product", ID: id, Body: body(map[string]any{"approve": false, "note": "Manual override"})})
	if records := history(id); records[0].Applied || *records[0].Outcome != "overridden" {
		t.Fatal("override history", records)
	}
	if _, err = restarted.PublicProduct(ctx, id); !errors.Is(err, store.ErrNotFound) {
		t.Fatal("override did not unpublish", err)
	}
	if _, err = s.AIReviewHistory(ctx, "buyer", store.Account{Kind: "personal", ID: 2}, id); !errors.Is(err, store.ErrForbidden) {
		t.Fatal("unrelated reader saw history", err)
	}
	// Capture assist at submit; a later settings change cannot approve it.
	must(t, policy.Save(ctx, marketreview.Modes{Tool: marketreview.Off, Store: marketreview.Assist}))
	assist := makeProduct()
	submit(assist)
	must(t, policy.Save(ctx, marketreview.Modes{Tool: marketreview.Off, Store: marketreview.Auto}))
	must(t, s.WorkAIReviews(ctx, 1))
	if records := history(assist); records[0].Applied || *records[0].Outcome != "reference" {
		t.Fatal(records)
	}
	// A manual review during provider I/O invalidates the exact running token.
	delayed := makeProduct()
	submit(delayed)
	classifier.hook = func() {
		invoke("admin", admin, store.LegacyRequest{Action: "review_product", ID: delayed, Body: body(map[string]any{"approve": false, "note": "Decision while provider is running"})})
	}
	must(t, s.WorkAIReviews(ctx, 1))
	if records := history(delayed); records[0].Applied || *records[0].Outcome != "overridden" {
		t.Fatal("late result overwrote human", records)
	}
	// Provider outages are persisted and bounded, including restart.
	retry := makeProduct()
	submit(retry)
	classifier.fail = true
	for i := 0; i < 3; i++ {
		must(t, restarted.WorkAIReviews(ctx, 1))
		_, err = db.Exec(`UPDATE lmm_store.products SET body=jsonb_set(body,'{ai_review,jobs,0,retry_at}','0'::jsonb) WHERE id=$1`, retry)
		must(t, err)
	}
	if records := history(retry); records[0].Status != "failed" || records[0].Attempts != 3 || *records[0].Outcome != "manual_required" {
		t.Fatal(records)
	}
	if ledger.effects != 0 {
		t.Fatal("listing review changed funds")
	}
}
