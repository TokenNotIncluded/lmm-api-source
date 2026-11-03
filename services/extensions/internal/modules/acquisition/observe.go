// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"unicode/utf8"
)

var opaqueLabel = regexp.MustCompile(`^[A-Za-z0-9]{41,}$`)
var labelPattern = regexp.MustCompile(`^[\pL\pN _.-]{1,80}$`)
var selfReportSecret = regexp.MustCompile(`(?i)(https?://|sk-[a-z0-9_-]{8,}|bearer\s+|(?:api[_-]?key|password|token|secret)\s*[:=])`)

// Label keeps short human campaign labels and drops anything that could be a
// credential or an opaque URL parameter.
func Label(value string) string {
	value = strings.TrimSpace(value)
	lower := strings.ToLower(value)
	if !labelPattern.MatchString(value) || strings.Contains(lower, "sk-") || strings.Contains(lower, "bearer") || strings.Count(value, ".") >= 2 {
		return ""
	}
	if opaqueLabel.MatchString(value) {
		return ""
	}
	return value
}

func Target(value string) string {
	switch value {
	case "/", "/guide", "/pricing", "/challenges", "/sign-up", "/sign-in":
		return value
	}
	return ""
}

func hostOf(value string) string {
	if len(value) > 2048 {
		return ""
	}
	u, err := url.Parse(value)
	if err != nil || (u.Scheme != "https" && u.Scheme != "http") || u.User != nil {
		return ""
	}
	host := strings.ToLower(u.Hostname())
	if len(host) > 253 || strings.ContainsAny(host, " @:%") || net.ParseIP(host) != nil || !strings.Contains(host, ".") || strings.HasSuffix(host, ".local") {
		return ""
	}
	return host
}

func randomID() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return ""
	}
	return hex.EncodeToString(b[:])
}

func visitorHash(cookie string) string {
	sum := sha256.Sum256([]byte(cookie))
	return hex.EncodeToString(sum[:])
}

func hexID(value string, size int) bool {
	if len(value) != size {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil && strings.ToLower(value) == value
}

type Input struct {
	ConsentVersion int    `json:"consent_version"`
	Landing        string `json:"landing"`
	Referrer       string `json:"referrer"`
	Source         string `json:"source"`
	Medium         string `json:"medium"`
	Campaign       string `json:"campaign"`
	Content        string `json:"content"`
	LinkID         string `json:"link_id"`
	Nonce          string `json:"nonce"`
	Consent        bool   `json:"consent"`
	Test           bool   `json:"test"`
}

type Visit struct {
	ConsentVersion int    `json:"consent_version"`
	ID             int64  `json:"id"`
	VisitorID      string `json:"-"`
	Nonce          string `json:"-"`
	LinkID         string `json:"link_id,omitempty"`
	Source         string `json:"source"`
	Medium         string `json:"medium,omitempty"`
	Campaign       string `json:"campaign,omitempty"`
	Content        string `json:"content,omitempty"`
	ReferrerHost   string `json:"referrer_host,omitempty"`
	Landing        string `json:"landing"`
	Evidence       string `json:"evidence"`
	CreatedAt      int64  `json:"created_at"`
}

// Normalize keeps no raw URL, query string, IP, user agent or credential.
func Normalize(input Input, ownHosts []string, now int64) (Visit, error) {
	if !input.Consent || input.Test || !hexID(strings.ToLower(input.Nonce), 32) || len(input.Nonce) != 32 {
		return Visit{}, ErrInvalid
	}
	landing, err := url.Parse(input.Landing)
	if err != nil || landing.IsAbs() || landing.Host != "" || strings.HasPrefix(landing.Path, "/oauth") || strings.Contains(landing.Path, "callback") || strings.Contains(landing.Path, "/return") {
		return Visit{}, ErrInvalid
	}
	page := Target(landing.Path)
	if page == "" {
		return Visit{}, ErrInvalid
	}
	for _, key := range []string{"code", "state", "session_id", "payment_intent", "trade_no", "redirect_status"} {
		if landing.Query().Has(key) {
			return Visit{}, ErrInvalid
		}
	}
	host := hostOf(input.Referrer)
	for _, own := range ownHosts {
		if strings.EqualFold(host, own) || strings.HasSuffix(host, "."+strings.ToLower(own)) {
			host = ""
		}
	}
	version := input.ConsentVersion
	if version == 0 {
		version = 1
	}
	if version < 1 || version > 2 {
		return Visit{}, ErrInvalid
	}
	result := Visit{ConsentVersion: version, Nonce: input.Nonce, Landing: page, ReferrerHost: host, CreatedAt: now, Source: "unknown", Evidence: "unavailable"}
	if source := Label(input.Source); source != "" {
		result.Source = source
		result.Evidence = "campaign_parameters"
		result.Medium = Label(input.Medium)
		result.Campaign = Label(input.Campaign)
		result.Content = Label(input.Content)
	} else if host != "" {
		result.Source = host
		result.Evidence = "browser_referrer"
	}
	return result, nil
}

// applyLink is shared by observations and read-only previews.
func applyLink(ctx context.Context, q querier, id string, visit *Visit) error {
	if len(id) != 32 {
		return nil
	}
	var source, medium, campaign, content string
	err := q.QueryRowContext(ctx, `SELECT source,medium,campaign,content FROM lmm_acquisition.links WHERE id=$1 AND deleted_at=0`, id).Scan(&source, &medium, &campaign, &content)
	if errors.Is(err, sql.ErrNoRows) {
		return nil
	}
	if err != nil {
		return err
	}
	visit.LinkID, visit.Source, visit.Medium = id, source, medium
	visit.Campaign, visit.Content, visit.Evidence = campaign, content, "promotion_link"
	return nil
}

// Observe records one consented visit. A shared browser never transfers
// another account's history.
func (s *Service) Observe(ctx context.Context, visitorID string, userID int64, input Input, ownHosts []string) (Visit, error) {
	now := s.now().Unix()
	visit, err := Normalize(input, ownHosts, now)
	if err != nil {
		return visit, err
	}
	if len(visitorID) != 64 {
		return visit, ErrInvalid
	}
	err = s.tx(ctx, func(ctx context.Context, tx *sql.Tx) error {
		if userID > 0 {
			var allowed bool
			var version int
			err := tx.QueryRowContext(ctx, `SELECT allowed,version FROM lmm_acquisition.consents WHERE user_id=$1`, userID).Scan(&allowed, &version)
			if err != nil && !errors.Is(err, sql.ErrNoRows) {
				return err
			}
			found := err == nil
			if found && !allowed || visit.ConsentVersion >= 2 && (!found || version < 2) {
				return ErrInvalid
			}
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.visitors(id,user_id,created_at) VALUES($1,0,$2) ON CONFLICT DO NOTHING`, visitorID, now); err != nil {
			return err
		}
		var owner int64
		if err := tx.QueryRowContext(ctx, `SELECT user_id FROM lmm_acquisition.visitors WHERE id=$1 FOR UPDATE`, visitorID).Scan(&owner); err != nil {
			return err
		}
		if owner != 0 && owner != userID {
			return ErrInvalid
		}
		if userID > 0 && owner == 0 {
			if _, err := tx.ExecContext(ctx, `UPDATE lmm_acquisition.visitors SET user_id=$2 WHERE id=$1 AND user_id=0`, visitorID, userID); err != nil {
				return err
			}
		}
		if err := applyLink(ctx, tx, input.LinkID, &visit); err != nil {
			return err
		}
		if userID > 0 && visit.ConsentVersion >= 2 {
			if _, err := tx.ExecContext(ctx, `UPDATE lmm_acquisition.accounts SET consent_version=2 WHERE user_id=$1 AND consent_version<2`, userID); err != nil {
				return err
			}
		}
		visit.VisitorID = visitorID
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.visits(visitor_id,nonce,consent_version,link_id,source,medium,campaign,content,referrer_host,landing,evidence,created_at)
 VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (visitor_id,nonce) DO NOTHING`,
			visitorID, visit.Nonce, visit.ConsentVersion, visit.LinkID, visit.Source, visit.Medium, visit.Campaign, visit.Content, visit.ReferrerHost, visit.Landing, visit.Evidence, visit.CreatedAt); err != nil {
			return err
		}
		return tx.QueryRowContext(ctx, `SELECT id,created_at FROM lmm_acquisition.visits WHERE visitor_id=$1 AND nonce=$2`, visitorID, visit.Nonce).Scan(&visit.ID, &visit.CreatedAt)
	})
	return visit, err
}

func (h *handlers) cookie(r *http.Request) string {
	c, err := r.Cookie(cookieName)
	if err != nil || !hexID(c.Value, 32) {
		return ""
	}
	return c.Value
}

func (h *handlers) setCookie(w http.ResponseWriter, value string, maxAge int) {
	http.SetCookie(w, &http.Cookie{Name: cookieName, Value: value, Path: "/", MaxAge: maxAge, Secure: h.s.secureCookie, HttpOnly: true, SameSite: http.SameSiteLaxMode})
}

// visit never reveals failures: analytics must not affect navigation.
func (h *handlers) visit(w http.ResponseWriter, r *http.Request) {
	noStore(w)
	var input Input
	if decode(w, r, &input, false) != nil || !input.Consent || input.Test {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	a, present, err := h.optionalUser(r)
	var userID int64
	if present && err == nil {
		if a.GetPlatformLevel() >= 5 {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		userID = a.GetUserId()
	} else if present {
		// An expired or unverifiable session must not become an anonymous visit
		// that a later account could claim.
		w.WriteHeader(http.StatusNoContent)
		return
	}
	cookie := h.cookie(r)
	if cookie == "" {
		cookie = randomID()
	}
	if cookie == "" {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	own := append([]string{strings.Split(r.Host, ":")[0]}, h.s.ownHosts...)
	if _, err := h.s.Observe(r.Context(), visitorHash(cookie), userID, input, own); err != nil {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	h.setCookie(w, cookie, 30*86400)
	w.WriteHeader(http.StatusNoContent)
}

type correctionHead struct {
	UserID    int64
	Revision  int64
	Source    string
	UpdatedAt int64
}

func lockCorrection(ctx context.Context, tx *sql.Tx, userID int64) (head correctionHead, err error) {
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.correction_heads(user_id) VALUES($1) ON CONFLICT DO NOTHING`, userID); err != nil {
		return
	}
	head.UserID = userID
	err = tx.QueryRowContext(ctx, `SELECT revision,source,updated_at FROM lmm_acquisition.correction_heads WHERE user_id=$1 FOR UPDATE`, userID).Scan(&head.Revision, &head.Source, &head.UpdatedAt)
	return
}

func setConsent(ctx context.Context, tx *sql.Tx, userID int64, allowed bool, now int64) error {
	_, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.consents(user_id,allowed,version,updated_at) VALUES($1,$2,2,$3)
 ON CONFLICT (user_id) DO UPDATE SET allowed=EXCLUDED.allowed,version=EXCLUDED.version,updated_at=EXCLUDED.updated_at`, userID, allowed, now)
	return err
}

// revokeAccount records the withdrawal and removes every derived record.
// Corrections lock first, then the analytics account and the preference.
func revokeAccount(ctx context.Context, tx *sql.Tx, userID int64, now int64) error {
	if _, err := lockCorrection(ctx, tx, userID); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.accounts(user_id,registration_source,created_at) VALUES($1,'unknown',$2) ON CONFLICT DO NOTHING`, userID, now); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `SELECT 1 FROM lmm_acquisition.accounts WHERE user_id=$1 FOR UPDATE`, userID); err != nil {
		return err
	}
	if err := setConsent(ctx, tx, userID, false, now); err != nil {
		return err
	}
	for _, table := range []string{"activities", "corrections", "correction_heads", "first_payments", "accounts"} {
		if _, err := tx.ExecContext(ctx, `DELETE FROM lmm_acquisition.`+table+` WHERE user_id=$1`, userID); err != nil {
			return err
		}
	}
	return nil
}

func (s *Service) RevokeVisitor(ctx context.Context, visitorID string) error {
	return s.tx(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var owner int64
		err := tx.QueryRowContext(ctx, `SELECT user_id FROM lmm_acquisition.visitors WHERE id=$1 FOR UPDATE`, visitorID).Scan(&owner)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		if owner > 0 {
			if err := revokeAccount(ctx, tx, owner, s.now().Unix()); err != nil {
				return err
			}
		}
		if _, err := tx.ExecContext(ctx, `DELETE FROM lmm_acquisition.visits WHERE visitor_id=$1`, visitorID); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, `DELETE FROM lmm_acquisition.visitors WHERE id=$1`, visitorID)
		return err
	})
}

func (s *Service) RevokeAccount(ctx context.Context, userID int64) error {
	if userID <= 0 {
		return nil
	}
	return s.tx(ctx, func(ctx context.Context, tx *sql.Tx) error { return revokeAccount(ctx, tx, userID, s.now().Unix()) })
}

// revoke is public: anonymous browsers withdraw by cookie; a signed-in user
// also withdraws the account even after the cookie expired.
func (h *handlers) revoke(w http.ResponseWriter, r *http.Request) {
	a, present, err := h.optionalUser(r)
	if present && err != nil {
		respond(w, nil, err)
		return
	}
	if cookie := h.cookie(r); cookie != "" {
		if err := h.s.RevokeVisitor(r.Context(), visitorHash(cookie)); err != nil {
			respond(w, nil, err)
			return
		}
	}
	if a != nil {
		if err := h.s.RevokeAccount(r.Context(), a.GetUserId()); err != nil {
			respond(w, nil, err)
			return
		}
	}
	// Keep the cookie on failure (above) so an anonymous user can retry.
	h.setCookie(w, "", -1)
	noStore(w)
	w.WriteHeader(http.StatusNoContent)
}

type cohortUser struct {
	RegisteredAt int64
	Staff        bool
	Deleted      bool
	Known        bool
}

func loadCohortUser(ctx context.Context, q querier, userID int64) (cohortUser, error) {
	var u cohortUser
	err := q.QueryRowContext(ctx, `SELECT registered_at,staff,deleted FROM lmm_acquisition.cohort WHERE user_id=$1`, userID).Scan(&u.RegisteredAt, &u.Staff, &u.Deleted)
	if errors.Is(err, sql.ErrNoRows) {
		return u, nil
	}
	u.Known = err == nil
	return u, err
}

func (s *Service) GrantConsent(ctx context.Context, userID int64) error {
	if userID <= 0 {
		return ErrInvalid
	}
	return s.tx(ctx, func(ctx context.Context, tx *sql.Tx) error {
		user, err := loadCohortUser(ctx, tx, userID)
		if err != nil {
			return err
		}
		cfg, err := loadConfig(ctx, tx, false)
		if err != nil {
			return err
		}
		now := s.now().Unix()
		source := "unknown"
		if user.Known && user.RegisteredAt < cfg.StartedAt {
			source = "historical_unrecorded"
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO lmm_acquisition.accounts(user_id,consent_version,registration_at,registration_source,registration_evidence,first_source,first_evidence,created_at,lookback_days)
 VALUES($1,2,$2,$3,'unavailable',$3,'unavailable',$4,$5) ON CONFLICT (user_id) DO UPDATE SET consent_version=2`, userID, user.RegisteredAt, source, now, cfg.LookbackDays); err != nil {
			return err
		}
		return setConsent(ctx, tx, userID, true, now)
	})
}

func (h *handlers) grant(w http.ResponseWriter, r *http.Request) {
	a, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if err := h.s.GrantConsent(r.Context(), a.GetUserId()); err != nil {
		respond(w, nil, err)
		return
	}
	respond(w, map[string]any{"allowed": true, "version": 2}, nil)
}

// SelfReport is explicit user testimony, never an input to attribution.
type SelfReport struct {
	Source    string `json:"source"`
	Detail    string `json:"detail"`
	UpdatedAt int64  `json:"updated_at"`
}

func (s *Service) ReadSelfReport(ctx context.Context, q querier, userID int64) (*SelfReport, error) {
	var v SelfReport
	err := q.QueryRowContext(ctx, `SELECT source,detail,updated_at FROM lmm_acquisition.self_reports WHERE user_id=$1 AND updated_at>=$2`, userID, s.now().Unix()-AccountDays*daySeconds).Scan(&v.Source, &v.Detail, &v.UpdatedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &v, nil
}

var selfSources = map[string]bool{"search": true, "community": true, "social": true, "documentation": true, "friend": true, "client": true, "ai": true, "other": true}

func (s *Service) SaveSelfReport(ctx context.Context, userID int64, source, detail string) error {
	detail = strings.TrimSpace(detail)
	if userID <= 0 || !selfSources[source] || utf8.RuneCountInString(detail) > 160 || strings.ContainsAny(detail, "\r\n\x00") || selfReportSecret.MatchString(detail) {
		return ErrInvalid
	}
	_, err := s.db.ExecContext(ctx, `INSERT INTO lmm_acquisition.self_reports(user_id,source,detail,updated_at) VALUES($1,$2,$3,$4)
 ON CONFLICT (user_id) DO UPDATE SET source=EXCLUDED.source,detail=EXCLUDED.detail,updated_at=EXCLUDED.updated_at`, userID, source, detail, s.now().Unix())
	return err
}

func (h *handlers) selfReport(w http.ResponseWriter, r *http.Request) {
	a, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	userID := a.GetUserId()
	switch r.Method {
	case http.MethodPut:
		var input struct {
			Source string `json:"source"`
			Detail string `json:"detail"`
		}
		if err := decode(w, r, &input, false); err != nil {
			respond(w, nil, err)
			return
		}
		if err := h.s.SaveSelfReport(r.Context(), userID, input.Source, input.Detail); err != nil {
			respond(w, nil, err)
			return
		}
	case http.MethodDelete:
		if _, err := h.s.db.ExecContext(r.Context(), `DELETE FROM lmm_acquisition.self_reports WHERE user_id=$1`, userID); err != nil {
			respond(w, nil, err)
			return
		}
	}
	value, err := h.s.ReadSelfReport(r.Context(), h.s.db, userID)
	respond(w, value, err)
}

var _ = fmt.Sprintf
