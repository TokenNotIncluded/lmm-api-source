// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Browser human-support views keep the original /api/assistant/support DTOs:
// numeric Web conversation IDs and zero-valued optional timestamps.
package assistant_services

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/lib/pq"
)

// ErrSupportIneligible is a forbidden appointment without a qualifying record;
// an immediate handoff stays available.
var ErrSupportIneligible = fmt.Errorf("%w: appointment requires a qualifying record", access.ErrForbidden)

// WebSupportRequest is the original Web request shape.
type WebSupportRequest struct {
	ID                int64  `json:"id"`
	UserID            int64  `json:"user_id"`
	ConversationID    int64  `json:"conversation_id"`
	Kind              string `json:"kind"`
	Status            string `json:"status"`
	Topic             string `json:"topic"`
	PreferredTime     string `json:"preferred_time"`
	ScheduledAt       int64  `json:"scheduled_at"`
	AssignedAdminID   int64  `json:"assigned_admin_id"`
	AssignedAdminName string `json:"assigned_admin_name"`
	CreatedAt         int64  `json:"created_at"`
	UpdatedAt         int64  `json:"updated_at"`
	AcceptedAt        int64  `json:"accepted_at"`
	ClosedAt          int64  `json:"closed_at"`
}

// WebSupportMessage matches the Web assistant history message view.
type WebSupportMessage struct {
	ID        int64  `json:"id"`
	Role      string `json:"role"`
	Content   string `json:"content"`
	CreatedAt int64  `json:"created_at"`
}

type WebSupportInput struct {
	ConversationID int64  `json:"conversation_id"`
	Kind           string `json:"kind"`
	Topic          string `json:"topic"`
	PreferredTime  string `json:"preferred_time"`
	ScheduledAt    int64  `json:"scheduled_at"`
}

// Requests linked to a Web conversation without a registered Core conversation
// use this local key; standalone requests use a per-request local key.
const webConversationPrefix = "web-conversation:"

type queryer interface {
	QueryRowContext(context.Context, string, ...any) *sql.Row
}

func webConversationID(ctx context.Context, q queryer, owner int64, key string) int64 {
	if raw, ok := strings.CutPrefix(key, webConversationPrefix); ok {
		id, err := strconv.ParseInt(raw, 10, 64)
		if err == nil && id > 0 {
			return id
		}
		return 0
	}
	if !strings.HasPrefix(key, "lmm_cv_") {
		return 0
	}
	var id int64
	if q.QueryRowContext(ctx, `SELECT conversation_id FROM lmm_assistant.delegation_conversations WHERE user_id=$1 AND core_conversation_id=$2`, owner, key).Scan(&id) != nil {
		return 0
	}
	return id
}

// WebSupportView converts the stored request; an unresolved key reports 0.
func webSupportView(ctx context.Context, q queryer, r SupportRequest) WebSupportRequest {
	v := WebSupportRequest{ID: r.ID, UserID: r.UserID, Kind: r.Kind, Status: r.Status, Topic: r.Topic, PreferredTime: r.PreferredTime, ScheduledAt: r.ScheduledAt, AssignedAdminName: r.AssignedAdminName, CreatedAt: r.CreatedAt, UpdatedAt: r.UpdatedAt}
	if r.AssignedAdminID != nil {
		v.AssignedAdminID = *r.AssignedAdminID
	}
	if r.AcceptedAt != nil {
		v.AcceptedAt = *r.AcceptedAt
	}
	if r.ClosedAt != nil {
		v.ClosedAt = *r.ClosedAt
	}
	v.ConversationID = webConversationID(ctx, q, r.UserID, r.CoreConversationID)
	return v
}

// conversationKeys verifies Web conversation ownership and returns the keys a
// request for it may be stored under: the registered Core ID first, if any.
func (s *Service) conversationKeys(ctx context.Context, q queryer, owner, conversation int64) ([]string, error) {
	var exists bool
	if err := q.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM lmm_assistant.conversations WHERE id=$1 AND user_id=$2)`, conversation, owner).Scan(&exists); err != nil {
		return nil, access.ErrUnavailable
	}
	if !exists {
		return nil, access.ErrNotFound
	}
	keys := []string{}
	var core string
	err := q.QueryRowContext(ctx, `SELECT core_conversation_id FROM lmm_assistant.delegation_conversations WHERE user_id=$1 AND conversation_id=$2 AND phase='registered' AND core_conversation_id IS NOT NULL`, owner, conversation).Scan(&core)
	if err == nil {
		keys = append(keys, core)
	} else if !errors.Is(err, sql.ErrNoRows) {
		return nil, access.ErrUnavailable
	}
	return append(keys, webConversationPrefix+strconv.FormatInt(conversation, 10)), nil
}

// WebSupportEligibility reports the Core-verified appointment eligibility.
func (s *Service) WebSupportEligibility(ctx context.Context, credential string) (bool, error) {
	if _, err := s.principal(ctx, credential); err != nil {
		return false, err
	}
	proof, err := s.eligibility(ctx, credential)
	return proof.Eligible, err
}

// WebSupportSelf returns the active request (conversation 0) or the latest
// request for one owned Web conversation; nil when there is none.
func (s *Service) WebSupportSelf(ctx context.Context, credential string, conversation int64) (*WebSupportRequest, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	if conversation < 0 {
		return nil, access.ErrInvalid
	}
	query := `SELECT ` + supportColumns + ` FROM lmm_assistant.platform_support WHERE user_id=$1 AND status IN('pending','accepted')`
	args := []any{p.UserID}
	if conversation > 0 {
		keys, err := s.conversationKeys(ctx, s.db, p.UserID, conversation)
		if err != nil {
			return nil, err
		}
		query = `SELECT ` + supportColumns + ` FROM lmm_assistant.platform_support WHERE user_id=$1 AND core_conversation_id=ANY($2::text[]) ORDER BY id DESC LIMIT 1`
		args = append(args, pq.Array(keys))
	}
	request, err := scanSupport(s.db.QueryRowContext(ctx, query, args...))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, access.ErrUnavailable
	}
	v := webSupportView(ctx, s.db, request)
	return &v, nil
}

// WebCreateSupport records a handoff or an eligible appointment request.
func (s *Service) WebCreateSupport(ctx context.Context, credential string, in WebSupportInput) (WebSupportRequest, bool, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return WebSupportRequest{}, false, err
	}
	if in.ConversationID < 0 || (in.Kind != "handoff" && in.Kind != "appointment") {
		return WebSupportRequest{}, false, access.ErrInvalid
	}
	// An existing active request is returned unchanged even when ineligible.
	receipt, eligible := "", true
	if in.Kind == "appointment" {
		proof, err := s.eligibility(ctx, credential)
		if err != nil {
			return WebSupportRequest{}, false, err
		}
		eligible = proof.Eligible
		if eligible {
			receipt = proof.ReceiptID
		}
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return WebSupportRequest{}, false, access.ErrUnavailable
	}
	defer tx.Rollback()
	var key string
	if in.ConversationID > 0 {
		keys, err := s.conversationKeys(ctx, tx, p.UserID, in.ConversationID)
		if err != nil {
			return WebSupportRequest{}, false, err
		}
		key = keys[0]
	} else {
		var nonce [8]byte
		if _, err := rand.Read(nonce[:]); err != nil {
			return WebSupportRequest{}, false, access.ErrUnavailable
		}
		key = "web-support:" + hex.EncodeToString(nonce[:])
	}
	request, created, err := s.createSupport(ctx, tx, p, key, in.Kind, BookInput{Topic: in.Topic, PreferredTime: in.PreferredTime, ScheduledAt: in.ScheduledAt}, receipt)
	if err != nil {
		if !eligible && errors.Is(err, access.ErrInvalid) {
			err = ErrSupportIneligible
		}
		return WebSupportRequest{}, false, err
	}
	v := webSupportView(ctx, tx, request)
	if tx.Commit() != nil {
		return WebSupportRequest{}, false, access.ErrUnavailable
	}
	return v, created, nil
}

// WebSupportDetail lets an administrator inspect pending queue metadata, but
// only the owner or the assigned administrator receives the transcript.
func (s *Service) WebSupportDetail(ctx context.Context, credential string, id int64) (WebSupportRequest, []WebSupportMessage, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return WebSupportRequest{}, nil, err
	}
	if id <= 0 {
		return WebSupportRequest{}, nil, access.ErrNotFound
	}
	request, err := scanSupport(s.db.QueryRowContext(ctx, `SELECT `+supportColumns+` FROM lmm_assistant.platform_support WHERE id=$1`, id))
	if errors.Is(err, sql.ErrNoRows) {
		return WebSupportRequest{}, nil, access.ErrNotFound
	}
	if err != nil {
		return WebSupportRequest{}, nil, access.ErrUnavailable
	}
	assigned := p.Level >= 5 && request.AssignedAdminID != nil && *request.AssignedAdminID == p.UserID
	if request.UserID != p.UserID && !(p.Level >= 5 && (request.Status == "pending" || assigned)) {
		return WebSupportRequest{}, nil, access.ErrForbidden
	}
	messages := []WebSupportMessage{}
	if request.UserID == p.UserID || assigned {
		var after int64
		for page := 0; page < 20; page++ {
			batch, err := s.SupportMessages(ctx, credential, id, after)
			if err != nil {
				return WebSupportRequest{}, nil, err
			}
			for _, m := range batch {
				messages = append(messages, WebSupportMessage{ID: m.ID, Role: m.Role, Content: m.Content, CreatedAt: m.CreatedAt})
				after = m.ID
			}
			if len(batch) < 50 {
				break
			}
		}
	}
	return webSupportView(ctx, s.db, request), messages, nil
}

// WebSendSupportMessage keeps the original optional client turn ID.
func (s *Service) WebSendSupportMessage(ctx context.Context, credential string, id int64, clientTurn, content string) (WebSupportMessage, error) {
	if clientTurn == "" {
		var nonce [12]byte
		if _, err := rand.Read(nonce[:]); err != nil {
			return WebSupportMessage{}, access.ErrUnavailable
		}
		clientTurn = hex.EncodeToString(nonce[:])
	}
	m, err := s.SendSupportMessage(ctx, credential, id, clientTurn, content)
	if err != nil {
		return WebSupportMessage{}, err
	}
	return WebSupportMessage{ID: m.ID, Role: m.Role, Content: m.Content, CreatedAt: m.CreatedAt}, nil
}

func (s *Service) WebAcceptSupport(ctx context.Context, credential string, id int64) (WebSupportRequest, error) {
	request, err := s.AcceptSupport(ctx, credential, id)
	if err != nil {
		return WebSupportRequest{}, err
	}
	return webSupportView(ctx, s.db, request), nil
}

func (s *Service) WebCloseSupport(ctx context.Context, credential string, id int64, cancel bool) (WebSupportRequest, error) {
	request, err := s.CloseSupport(ctx, credential, id, cancel)
	if err != nil {
		return WebSupportRequest{}, err
	}
	return webSupportView(ctx, s.db, request), nil
}
