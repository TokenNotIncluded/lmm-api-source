// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"database/sql"
	"net/http"
	"net/url"
	"strings"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

const (
	stepCreateAPIKey    = "create_api_key"
	stepInstallClient   = "install_client"
	stepConfigureClient = "configure_client"
	stepFirstResponse   = "first_successful_response"
)

type onboardingStep struct {
	ID          string `json:"id"`
	Status      string `json:"status"`
	CompletedAt int64  `json:"completed_at,omitempty"`
}

type onboardingView struct {
	Eligibility map[string]any   `json:"eligibility"`
	Status      string           `json:"status"`
	CurrentStep string           `json:"current_step,omitempty"`
	Steps       []onboardingStep `json:"steps"`
	CompletedAt int64            `json:"completed_at,omitempty"`
}

type onboardingTodo struct {
	keyProvenAt, installedAt, configuredAt, completedAt, createdAt int64
}

// Eligibility uses the Core platform level (L1+ or administrator). Core's
// developer-access grant is not part of the identity RPC response.
func eligibility(actor *pb.AuthorizeResponse) map[string]any {
	level := actor.GetPlatformLevel()
	e := map[string]any{"eligible": level >= 1, "developer_access_granted": level >= 5, "trust_level": level}
	if level < 1 {
		e["reason"] = "L1_REQUIRED"
	}
	return e
}

func (t onboardingTodo) view(actor *pb.AuthorizeResponse) onboardingView {
	done := func(ok bool, at int64) onboardingStep {
		if ok {
			return onboardingStep{Status: "completed", CompletedAt: at}
		}
		return onboardingStep{Status: "pending"}
	}
	key := t.keyProvenAt > 0
	installed := key && t.installedAt > 0
	configured := installed && t.configuredAt > 0
	// The legacy last-API-activity fact is Core usage data; only progress that
	// was already completed before migration is retained.
	first := configured && t.completedAt > 0
	steps := []onboardingStep{done(key, t.keyProvenAt), done(installed, t.installedAt), done(configured, t.configuredAt), done(first, t.completedAt)}
	current := ""
	for i, id := range []string{stepCreateAPIKey, stepInstallClient, stepConfigureClient, stepFirstResponse} {
		steps[i].ID = id
		if current == "" && steps[i].Status != "completed" {
			current = id
		}
	}
	status, completed := "in_progress", int64(0)
	if first {
		status, completed = "completed", t.completedAt
	}
	return onboardingView{Eligibility: eligibility(actor), Status: status, CurrentStep: current, Steps: steps, CompletedAt: completed}
}

func (s *Service) loadTodo(ctx context.Context, q interface {
	QueryRowContext(context.Context, string, ...any) *sql.Row
}, userID int64, lock bool) (onboardingTodo, error) {
	var t onboardingTodo
	query := `SELECT key_proven_at,client_installed_at,client_configured_at,completed_at,created_at FROM lmm_community.onboarding_todos WHERE user_id=$1`
	if lock {
		query += " FOR UPDATE"
	}
	err := q.QueryRowContext(ctx, query, userID).Scan(&t.keyProvenAt, &t.installedAt, &t.configuredAt, &t.completedAt, &t.createdAt)
	return t, err
}

// GET and PATCH /api/user/self/onboarding/todo. PATCH is a refresh and accepts
// no completion flags. L0 users receive an unavailable state and no row.
func (h *handlers) onboardingTodo(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if actor.GetPlatformLevel() < 1 {
		respond(w, onboardingView{Eligibility: eligibility(actor), Status: "unavailable", Steps: []onboardingStep{}}, nil)
		return
	}
	now := h.s.now().Unix()
	if _, err = h.s.db.ExecContext(r.Context(), `INSERT INTO lmm_community.onboarding_todos(user_id,created_at,updated_at) VALUES($1,$2,$2) ON CONFLICT (user_id) DO NOTHING`, actor.GetUserId(), now); err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	t, err := h.s.loadTodo(r.Context(), h.s.db, actor.GetUserId(), false)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	respond(w, t.view(actor), nil)
}

// POST /api/onboarding/todo/proof is called by a configured client with the
// user's API key. Core validates the key; a valid key also proves step one.
func (h *handlers) onboardingProof(w http.ResponseWriter, r *http.Request) {
	actor, err := h.auth.VerifyAPIKey(r.Context(), r)
	if err != nil {
		respond(w, nil, authFailure(err))
		return
	}
	var proof struct {
		Step    string `json:"step"`
		Client  string `json:"client"`
		BaseURL string `json:"base_url"`
		Group   string `json:"group"`
	}
	invalid := &apiError{http.StatusUnprocessableEntity, "L1_ONBOARDING_INVALID_PROOF", "invalid onboarding proof"}
	if err = decode(w, r, 4<<10, &proof, false); err != nil {
		respond(w, nil, invalid)
		return
	}
	proof.Step, proof.Client = strings.TrimSpace(proof.Step), strings.TrimSpace(proof.Client)
	proof.BaseURL, proof.Group = strings.TrimSpace(proof.BaseURL), strings.TrimSpace(proof.Group)
	if proof.Step != stepInstallClient && proof.Step != stepConfigureClient {
		respond(w, nil, &apiError{http.StatusUnprocessableEntity, "L1_ONBOARDING_INVALID_STEP", "invalid L1 onboarding step"})
		return
	}
	if n := len([]rune(proof.Client)); n == 0 || n > 64 {
		respond(w, nil, invalid)
		return
	}
	if proof.Step == stepConfigureClient {
		u, e := url.Parse(proof.BaseURL)
		if proof.BaseURL == "" || len([]rune(proof.BaseURL)) > 256 || proof.Group == "" || len(proof.Group) > 64 || e != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" || u.User != nil {
			respond(w, nil, invalid)
			return
		}
	}
	if actor.GetPlatformLevel() < 1 {
		respond(w, nil, &apiError{http.StatusForbidden, "L1_ONBOARDING_NOT_ELIGIBLE", "L1 onboarding is only available to users with approved developer access"})
		return
	}
	now := h.s.now().Unix()
	var t onboardingTodo
	err = h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		if _, e := tx.ExecContext(ctx, `INSERT INTO lmm_community.onboarding_todos(user_id,created_at,updated_at) VALUES($1,$2,$2) ON CONFLICT (user_id) DO NOTHING`, actor.GetUserId(), now); e != nil {
			return e
		}
		var e error
		if t, e = h.s.loadTodo(ctx, tx, actor.GetUserId(), true); e != nil {
			return e
		}
		if t.keyProvenAt == 0 {
			t.keyProvenAt = now
		}
		switch proof.Step {
		case stepInstallClient:
			if t.installedAt == 0 {
				t.installedAt = now
			}
		case stepConfigureClient:
			if t.configuredAt == 0 {
				if t.installedAt == 0 {
					return &apiError{http.StatusConflict, "L1_ONBOARDING_OUT_OF_ORDER", "L1 onboarding steps must be completed in order"}
				}
				t.configuredAt = now
			}
		}
		_, e = tx.ExecContext(ctx, `UPDATE lmm_community.onboarding_todos SET key_proven_at=$2,client_installed_at=$3,client_configured_at=$4,updated_at=$5 WHERE user_id=$1`, actor.GetUserId(), t.keyProvenAt, t.installedAt, t.configuredAt, now)
		return e
	})
	if err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, t.view(actor), nil)
}
