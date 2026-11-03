// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.

// Package acquisition serves the legacy user-acquisition (source attribution)
// Web contract from its own lmm_acquisition database. Identity and platform
// roles come from Core on every request. It records analytics only: it never
// reads or writes balances, payments, rewards or Core storage.
//
// Semantics adapted from api.lmm.best apps/api-go/{model,controller,router}/acquisition*.go
// (AGPL-3.0-or-later; QuantumNous/new-api derived).
package acquisition

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
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Schema is a fresh-install schema, installed offline by freshinstall.
//
//go:embed schema.sql
var Schema string

const contractVersion = 1

const (
	RawDays     = 90
	AccountDays = 365
	daySeconds  = int64(86400)
	cookieName  = "lmm_acquisition"
)

// Verifier is satisfied by webcompat.Authenticator; it validates the live
// session through Core on every request.
type Verifier interface {
	Verify(context.Context, *http.Request) (*pb.AuthorizeResponse, error)
}

type UserReader interface {
	Users(context.Context, []int64) (map[int64]community.CoreUserInfo, error)
}

type Service struct {
	UserRead     UserReader
	db           *sql.DB
	secureCookie bool
	ownHosts     []string
	now          func() time.Time
}

// New borrows the dedicated pool opened by composition and checks the schema
// contract. It runs no DDL. publicOrigin's host is treated as an own host so
// internal navigation is never recorded as an external referrer.
func New(ctx context.Context, db *sql.DB, publicOrigin string, secureCookie bool) (*Service, error) {
	if db == nil {
		return nil, store.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var version int
	if err := db.QueryRowContext(ctx, `SELECT version FROM lmm_acquisition.contract WHERE singleton`).Scan(&version); err != nil || version != contractVersion {
		return nil, errors.New("acquisition storage contract unavailable")
	}
	var configured bool
	if err := db.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM lmm_acquisition.config WHERE singleton)`).Scan(&configured); err != nil || !configured {
		return nil, errors.New("acquisition configuration unavailable")
	}
	hosts := []string{"lmm.best"}
	if origin := strings.TrimSpace(publicOrigin); origin != "" {
		if host := hostOf(origin); host != "" {
			hosts = append(hosts, host)
		}
	}
	return &Service{db: db, secureCookie: secureCookie, ownHosts: hosts, now: time.Now}, nil
}

// Owns reports the public Web paths served by this module.
func Owns(path string) bool {
	return strings.HasPrefix(path, "/api/acquisition/") || strings.HasPrefix(path, "/api/admin/acquisition/")
}

// Register mounts the exact legacy routes on mux.
func (s *Service) Register(mux *http.ServeMux, auth Verifier) {
	h := &handlers{s: s, auth: auth}
	mux.HandleFunc("POST /api/acquisition/visit", h.visit)
	mux.HandleFunc("DELETE /api/acquisition/consent", h.revoke)
	mux.HandleFunc("POST /api/acquisition/consent", h.grant)
	mux.HandleFunc("GET /api/acquisition/self-report", h.selfReport)
	mux.HandleFunc("PUT /api/acquisition/self-report", h.selfReport)
	mux.HandleFunc("DELETE /api/acquisition/self-report", h.selfReport)

	// Administrative acquisition handlers were not completed before handoff.
	// Keep their authorization boundaries and return an explicit failure until B11
	// supplies the real implementations; never serve fabricated analytics.
	mux.HandleFunc("GET /api/admin/acquisition/links/{id}/preview", h.admin(permRead, h.previewLink))
	mux.HandleFunc("GET /api/admin/acquisition/cost", h.admin(permRead, unavailableAdmin))
	mux.HandleFunc("PUT /api/admin/acquisition/cost", h.admin(permWrite, h.saveCost))
	mux.HandleFunc("GET /api/admin/acquisition/links", h.admin(permRead, h.links))
	mux.HandleFunc("POST /api/admin/acquisition/links", h.admin(permWrite, h.saveLink))
	mux.HandleFunc("DELETE /api/admin/acquisition/links/{id}", h.admin(permWrite, h.deleteLink))
	mux.HandleFunc("POST /api/admin/acquisition/activity/rebuild", h.admin(permWrite, unavailableAdmin))
	mux.HandleFunc("GET /api/admin/acquisition/users/export", h.admin(permExport, unavailableAdmin))
	mux.HandleFunc("GET /api/admin/acquisition/users/{id}/corrections", h.admin(permDetails, h.corrections))
	mux.HandleFunc("POST /api/admin/acquisition/users/{id}/corrections", h.admin(permDetailsWrite, h.saveCorrection))
	mux.HandleFunc("GET /api/admin/acquisition/users", h.admin(permDetails, unavailableAdmin))
	mux.HandleFunc("GET /api/admin/acquisition/users/{id}", h.admin(permDetails, h.userDetail))
	mux.HandleFunc("PUT /api/admin/acquisition/lookback", h.admin(permWrite, h.lookback))
	mux.HandleFunc("GET /api/admin/acquisition/visitors", h.admin(permRead, h.visitors))
	mux.HandleFunc("GET /api/admin/acquisition/funnel", h.admin(permRead, unavailableAdmin))
	mux.HandleFunc("GET /api/admin/acquisition/report", h.admin(permRead, unavailableAdmin))
}

func unavailableAdmin(w http.ResponseWriter, r *http.Request, a actor) {
	respond(w, nil, &apiError{http.StatusNotImplemented, "ACQUISITION_NOT_IMPLEMENTED", "Acquisition administration is not implemented"})
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
	ErrInvalid     = &apiError{http.StatusBadRequest, "ACQUISITION_INVALID_INPUT", "invalid acquisition data"}
	errUnavailable = &apiError{http.StatusServiceUnavailable, "ACQUISITION_UNAVAILABLE", "Acquisition data is temporarily unavailable"}
	errNotFound    = &apiError{http.StatusNotFound, "ACQUISITION_NOT_FOUND", "Not found"}
	errForbidden   = &apiError{http.StatusForbidden, "ACQUISITION_ACCESS_DENIED", "This operation is not available to this account"}
	errLogin       = &apiError{http.StatusUnauthorized, "ACQUISITION_LOGIN_REQUIRED", "Authentication required"}
	errConflict    = &apiError{http.StatusConflict, "ACQUISITION_CORRECTION_CONFLICT", "Source correction changed; reload before saving"}
	errLimit       = &apiError{http.StatusBadRequest, "ACQUISITION_LIMIT", "Narrow the date range (cohort limit exceeded)"}
	errNoActivity  = &apiError{http.StatusServiceUnavailable, "ACQUISITION_ACTIVITY_SOURCE_UNAVAILABLE", "Activity observation needs a Core usage read that is not available yet"}
)

func authFailure(err error) error {
	switch {
	case errors.Is(err, store.ErrUnauthorized):
		return errLogin
	case errors.Is(err, store.ErrForbidden):
		return errForbidden
	case errors.Is(err, store.ErrInvalid):
		return ErrInvalid
	}
	return errUnavailable
}

// Platform levels from Core: 5 administrator, 6 root. Core's default policy
// grants acquisition read/write to administrators and every action to root;
// details/export have no administrator default, so they require root here.
type permission int

const (
	permRead permission = iota
	permWrite
	permDetails
	permDetailsWrite
	permExport
)

func allowed(level int32, p permission) bool {
	switch p {
	case permRead, permWrite:
		return level == 5 || level == 6
	default:
		return level == 6
	}
}

type actor struct {
	id    int64
	level int32
}

func (h *handlers) user(r *http.Request) (*pb.AuthorizeResponse, error) {
	a, err := h.auth.Verify(r.Context(), r)
	if err != nil {
		return nil, authFailure(err)
	}
	return a, nil
}

// optionalUser mirrors TryUserAuth: no Authorization header means anonymous.
// A present but invalid credential is reported to the caller.
func (h *handlers) optionalUser(r *http.Request) (*pb.AuthorizeResponse, bool, error) {
	present := false
	for key := range r.Header {
		if strings.EqualFold(key, "Authorization") {
			present = true
		}
	}
	if !present {
		return nil, false, nil
	}
	a, err := h.user(r)
	return a, true, err
}

func (h *handlers) admin(p permission, fn func(http.ResponseWriter, *http.Request, actor)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		a, err := h.user(r)
		if err != nil {
			respond(w, nil, err)
			return
		}
		if !allowed(a.GetPlatformLevel(), p) {
			respond(w, nil, errForbidden)
			return
		}
		fn(w, r, actor{id: a.GetUserId(), level: a.GetPlatformLevel()})
	}
}

func noStore(w http.ResponseWriter) {
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
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
			if errors.Is(err, sql.ErrNoRows) {
				e = errNotFound
			}
		}
		code = e.status
		body = map[string]any{"success": false, "code": e.code, "message": e.message}
	}
	encoded, _ := json.Marshal(body)
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	noStore(w)
	w.WriteHeader(code)
	_, _ = w.Write(append(encoded, '\n'))
}

// decode reads one bounded JSON object (legacy 4096-byte body limit).
func decode(w http.ResponseWriter, r *http.Request, out any, allowEmpty bool) error {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096))
	if err := dec.Decode(out); err != nil {
		if allowEmpty && errors.Is(err, io.EOF) {
			return nil
		}
		return ErrInvalid
	}
	if err := dec.Decode(new(any)); !errors.Is(err, io.EOF) {
		return ErrInvalid
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

type config struct {
	StartedAt                int64
	LookbackDays             int
	LastCleanupAt            int64
	PaymentSnapshotUpdatedAt int64
	PaymentSnapshotStatus    string
}

type querier interface {
	QueryRowContext(context.Context, string, ...any) *sql.Row
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
	ExecContext(context.Context, string, ...any) (sql.Result, error)
}

func loadConfig(ctx context.Context, q querier, lock bool) (config, error) {
	var c config
	query := `SELECT started_at,lookback_days,last_cleanup_at,payment_snapshot_updated_at,payment_snapshot_status FROM lmm_acquisition.config WHERE singleton`
	if lock {
		query += " FOR UPDATE"
	}
	err := q.QueryRowContext(ctx, query).Scan(&c.StartedAt, &c.LookbackDays, &c.LastCleanupAt, &c.PaymentSnapshotUpdatedAt, &c.PaymentSnapshotStatus)
	return c, err
}
