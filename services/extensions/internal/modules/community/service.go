// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.

// Package community serves the legacy community pages that have no Core
// backend: AI directory, profile share links, L1 onboarding progress and the
// signal game. It owns only its dedicated lmm_community database. Identity and
// platform roles always come from Core; it never moves money.
package community

import (
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Schema is a fresh-install schema, installed offline by freshinstall.
//
//go:embed schema.sql
var Schema string

const contractVersion = 1

// Verifier is satisfied by webcompat.Authenticator. Both methods validate the
// live credential through Core on every request.
type Verifier interface {
	Verify(context.Context, *http.Request) (*pb.AuthorizeResponse, error)
	VerifyAPIKey(context.Context, *http.Request) (*pb.AuthorizeResponse, error)
}

type Service struct {
	db           *sql.DB
	origin       string
	now          func() time.Time
	core         *CoreRead
	rankingCache rankingCache
}

// Options supplies the optional Core read port. Without it, usage-backed pages
// fail explicitly. Paid placements remain disabled until commerce is implemented.
type Options struct {
	Core *CoreRead
}

// New borrows the dedicated pool opened by composition and checks the schema
// contract. It runs no DDL.
func New(ctx context.Context, db *sql.DB, publicOrigin string, opts ...Options) (*Service, error) {
	if db == nil {
		return nil, store.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var version int
	var err error
	if err = db.QueryRowContext(ctx, `SELECT version FROM lmm_community.contract WHERE singleton`).Scan(&version); err != nil || version != contractVersion {
		return nil, errors.New("community storage contract unavailable")
	}
	s := &Service{db: db, origin: strings.TrimRight(publicOrigin, "/"), now: time.Now}
	if len(opts) == 1 {
		s.core = opts[0].Core
	}
	return s, nil
}

// Owns reports the public Web paths served by this module.
func Owns(path string) bool {
	switch {
	case path == "/api/ai-directory", strings.HasPrefix(path, "/api/ai-directory/"),
		strings.HasPrefix(path, "/api/share/profile/"),
		path == "/api/user/self/profile-share",
		path == "/api/user/self/onboarding/todo", path == "/api/onboarding/todo/proof",
		strings.HasPrefix(path, "/api/games/signal/"),
		path == "/api/rankings", path == "/api/rankings/users":
		return true
	}
	return false
}

// Register mounts the Web routes on mux. auth must be non-nil.
func (s *Service) Register(mux *http.ServeMux, auth Verifier) {
	h := &handlers{s: s, auth: auth}
	mux.HandleFunc("GET /api/ai-directory", h.directoryLinks)
	mux.HandleFunc("PUT /api/ai-directory/links", h.updateDirectoryLinks)
	mux.HandleFunc("GET /api/ai-directory/ads", h.listAds)
	mux.HandleFunc("GET /api/ai-directory/ads/quote", h.quoteAd)
	mux.HandleFunc("GET /api/ai-directory/ads/mine", h.myAds)
	mux.HandleFunc("POST /api/ai-directory/ads", h.createAd)
	mux.HandleFunc("POST /api/ai-directory/ads/{id}/hide", h.hideAd)

	mux.HandleFunc("GET /api/user/self/profile-share", h.getProfileShare)
	mux.HandleFunc("POST /api/user/self/profile-share", h.enableProfileShare)
	mux.HandleFunc("DELETE /api/user/self/profile-share", h.disableProfileShare)
	mux.HandleFunc("GET /api/share/profile/{token}", h.profileShareSVG)

	mux.HandleFunc("GET /api/user/self/onboarding/todo", h.onboardingTodo)
	mux.HandleFunc("PATCH /api/user/self/onboarding/todo", h.onboardingTodo)
	mux.HandleFunc("POST /api/onboarding/todo/proof", h.onboardingProof)

	mux.HandleFunc("GET /api/games/signal/daily", h.signalDaily)
	mux.HandleFunc("POST /api/games/signal/attempts", h.signalBegin)
	mux.HandleFunc("POST /api/games/signal/finish", h.signalFinish)
	mux.HandleFunc("GET /api/games/signal/leaderboard", h.signalLeaderboard)
	mux.HandleFunc("GET /api/games/signal/records", h.signalRecords)
	mux.HandleFunc("POST /api/games/signal/records", h.signalSave)

	mux.HandleFunc("GET /api/rankings", h.rankings)
	mux.HandleFunc("GET /api/rankings/users", h.userRankings)
}

type handlers struct {
	s    *Service
	auth Verifier
}

type apiError struct {
	status        int
	code, message string
}

func (e *apiError) Error() string { return e.message }

var (
	errUnavailable = &apiError{http.StatusServiceUnavailable, "COMMUNITY_UNAVAILABLE", "The service is temporarily unavailable"}
	errInvalid     = &apiError{http.StatusBadRequest, "COMMUNITY_INVALID_INPUT", "Invalid request"}
	errNotFound    = &apiError{http.StatusNotFound, "COMMUNITY_NOT_FOUND", "Not found"}
	errForbidden   = &apiError{http.StatusForbidden, "COMMUNITY_ACCESS_DENIED", "This operation is not available to this account"}
)

func authFailure(err error) error {
	switch {
	case errors.Is(err, store.ErrUnauthorized):
		return &apiError{http.StatusUnauthorized, "COMMUNITY_LOGIN_REQUIRED", "Sign in first"}
	case errors.Is(err, store.ErrForbidden):
		return errForbidden
	case errors.Is(err, store.ErrInvalid):
		return errInvalid
	}
	return errUnavailable
}

func (h *handlers) user(r *http.Request) (*pb.AuthorizeResponse, error) {
	actor, err := h.auth.Verify(r.Context(), r)
	if err != nil {
		return nil, authFailure(err)
	}
	return actor, nil
}

// admin requires a fresh Core platform role of administrator (5) or root (6).
func (h *handlers) admin(r *http.Request) (*pb.AuthorizeResponse, error) {
	actor, err := h.user(r)
	if err != nil {
		return nil, err
	}
	if actor.GetPlatformLevel() < 5 {
		return nil, errForbidden
	}
	return actor, nil
}

// respond keeps the legacy success/message/data envelope. Internal errors never
// leak database detail.
func respond(w http.ResponseWriter, value any, err error) {
	code := http.StatusOK
	body := map[string]any{"success": true, "message": "", "data": value}
	if err != nil {
		var e *apiError
		if !errors.As(err, &e) {
			e = errUnavailable
		}
		code = e.status
		body = map[string]any{"success": false, "code": e.code, "message": e.message}
	}
	encoded, _ := json.Marshal(body)
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(code)
	_, _ = w.Write(append(encoded, '\n'))
}

// decode reads one bounded JSON object, rejecting unknown fields and trailing
// data. An empty body is accepted when allowEmpty is true.
func decode(w http.ResponseWriter, r *http.Request, limit int64, out any, allowEmpty bool) error {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, limit))
	dec.DisallowUnknownFields()
	if err := dec.Decode(out); err != nil {
		if allowEmpty && errors.Is(err, io.EOF) {
			return nil
		}
		return errInvalid
	}
	if err := dec.Decode(new(any)); !errors.Is(err, io.EOF) {
		return errInvalid
	}
	return nil
}

func (s *Service) tx(ctx context.Context, fn func(context.Context, *sql.Tx) error) error {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err = fn(ctx, tx); err != nil {
		return err
	}
	return tx.Commit()
}
