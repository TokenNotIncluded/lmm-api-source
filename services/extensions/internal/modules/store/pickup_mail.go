// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/mail"
	"net/url"
	"strings"
	"sync"
	"time"
)

type pickupMailConfig struct {
	sender *PickupSMTP
	aead   cipher.AEAD
	origin string
	mu     sync.Mutex
	after  PickupMailCursor
}

type pickupMailPayload struct {
	CreatedAt int64  `json:"created_at"`
	Recipient string `json:"recipient"`
	Token     string `json:"token"`
	URL       string `json:"url"`
}

func (pickupMailPayload) String() string     { return "[private store pickup mail payload]" }
func (m pickupMailPayload) GoString() string { return m.String() }

type pickupMailRecord struct {
	Kind          string `json:"kind"`
	OrderID       string `json:"order_id"`
	State         string `json:"state"`
	Attempt       string `json:"attempt,omitempty"`
	StartedAt     int64  `json:"started_at,omitempty"`
	NextAttemptAt int64  `json:"next_attempt_at,omitempty"`
	SMTPCode      int    `json:"smtp_code,omitempty"`
	Ciphertext    string `json:"ciphertext"`
}

// WithPickupMail requires a dedicated private 32-byte mail key and a configured
// HTTPS site origin, never a request Host header. Call only during composition.
// Existing ciphertext requires the same key after restart. No SMTP is attempted.
func (s *Service) WithPickupMail(sender *PickupSMTP, key []byte, origin string) error {
	u, err := url.Parse(origin)
	if sender == nil || len(key) != 32 || err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Opaque != "" || (u.Path != "" && u.Path != "/") {
		return ErrInvalid
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return ErrInvalid
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return ErrUnavailable
	}
	s.pickupMail = &pickupMailConfig{sender: sender, aead: aead, origin: strings.TrimSuffix(origin, "/")}
	return nil
}

func pickupEmailValid(email string) bool {
	if email == "" || len(email) > 254 || strings.TrimSpace(email) != email || strings.ContainsAny(email, "\r\n\x00") {
		return false
	}
	for _, c := range email {
		if c <= 32 || c >= 127 {
			return false
		}
	}
	address, err := mail.ParseAddress(email)
	return err == nil && address.Name == "" && address.Address == email
}
func pickupMailID(order string) string { return ID("store-pickup-mail-v1", order) }
func pickupMailAAD(shop, order string) []byte {
	return []byte("lmm-store-pickup-mail-v1\x00" + shop + "\x00" + order)
}

func (s *Service) openPickupMail(tx Tx, shop, order string) (pickupMailRecord, pickupMailPayload, error) {
	var record pickupMailRecord
	var payload pickupMailPayload
	if err := tx.Get("commands", pickupMailID(order), &record); err != nil {
		return record, payload, err
	}
	if s.pickupMail == nil {
		return record, payload, ErrUnavailable
	}
	if record.Kind != "store-pickup-mail-v1" || record.OrderID != order {
		return record, payload, ErrUnavailable
	}
	raw, err := base64.RawURLEncoding.DecodeString(record.Ciphertext)
	n := s.pickupMail.aead.NonceSize()
	if err != nil || len(raw) < n {
		return record, payload, ErrUnavailable
	}
	plain, err := s.pickupMail.aead.Open(nil, raw[:n], raw[n:], pickupMailAAD(shop, order))
	if err != nil {
		return record, payload, ErrUnavailable
	}
	defer clear(plain)
	if json.Unmarshal(plain, &payload) != nil || payload.CreatedAt <= 0 || !pickupEmailValid(payload.Recipient) || !strings.HasSuffix(payload.URL, "/store/claim/"+payload.Token) {
		return record, pickupMailPayload{}, ErrUnavailable
	}
	return record, payload, nil
}

// This runs in the SAME shop transaction as order creation and inventory.
func (s *Service) preparePickupMail(tx Tx, shop string, order Order, email string) error {
	if email == "" {
		return nil
	}
	if s.pickupMail == nil {
		return ErrUnavailable
	}
	var secret [32]byte
	if _, err := rand.Read(secret[:]); err != nil {
		return ErrUnavailable
	}
	token := order.ID + "." + hex.EncodeToString(secret[:])
	createdAt := time.Now().Unix()
	if order.Legacy != nil && order.Legacy.CreatedAt > 0 {
		createdAt = order.Legacy.CreatedAt
	}
	payload := pickupMailPayload{CreatedAt: createdAt, Recipient: email, Token: token, URL: s.pickupMail.origin + "/store/claim/" + token}
	plain, err := json.Marshal(payload)
	if err != nil {
		return ErrUnavailable
	}
	defer clear(plain)
	nonce := make([]byte, s.pickupMail.aead.NonceSize())
	if _, err = rand.Read(nonce); err != nil {
		return ErrUnavailable
	}
	ciphertext := s.pickupMail.aead.Seal(nonce, nonce, plain, pickupMailAAD(shop, order.ID))
	record := pickupMailRecord{Kind: "store-pickup-mail-v1", OrderID: order.ID, State: "pending", Ciphertext: base64.RawURLEncoding.EncodeToString(ciphertext)}
	if err := tx.Put("commands", pickupMailID(order.ID), record); err != nil {
		return err
	}
	// Only the bearer digest is kept in the pickup record. The recoverable token
	// lives exclusively in the encrypted mail payload, never in the order DTO.
	return tx.Put("commands", ID("store-pickup-v1", order.ID), pickupLink{Hash: ID(token)})
}

func (s *Service) checkPickupEmail(tx Tx, shop string, order Order, email string) error {
	if order.Legacy == nil || !order.Legacy.EmailPickupLink {
		if email != "" {
			return ErrConflict
		}
		return nil
	}
	_, payload, err := s.openPickupMail(tx, shop, order.ID)
	if err != nil {
		return err
	}
	if payload.Recipient != email {
		return ErrConflict
	}
	return nil
}

// DispatchPickupMail claims under the shop's PostgreSQL transaction lock before
// touching SMTP. Final updates compare the immutable ciphertext and attempt.
func (s *Service) DispatchPickupMail(ctx context.Context, shop, orderID string) (string, error) {
	if s.pickupMail == nil {
		return "", ErrUnavailable
	}
	var claimed bool
	var record pickupMailRecord
	var payload pickupMailPayload
	err := s.repo.Within(ctx, shop, func(tx Tx) error {
		var err error
		record, payload, err = s.openPickupMail(tx, shop, orderID)
		if err != nil {
			return err
		}
		now := time.Now().Unix()
		switch record.State {
		case "accepted_by_smtp", "rejected", "unknown", "canceled":
			return nil
		case "dispatching":
			if record.StartedAt <= 0 || record.Attempt == "" {
				return ErrUnavailable
			}
			if now-record.StartedAt >= 30 {
				record.State = "unknown"
				return tx.Put("commands", pickupMailID(orderID), record)
			}
			return nil
		case "retry_wait":
			if now < record.NextAttemptAt {
				return nil
			}
		case "pending":
		default:
			return ErrUnavailable
		}
		var order Order
		if err := tx.Get("orders", orderID, &order); err != nil {
			return err
		}
		if order.Legacy == nil || !order.Legacy.EmailPickupLink {
			return ErrConflict
		}
		if order.State == Cancelled || order.State == Refunded {
			record.State = "canceled"
			return tx.Put("commands", pickupMailID(orderID), record)
		}
		if order.RefundState == "requested" || (order.State != Paid && order.State != Fulfilled && order.State != SettlementPending && order.State != Settled) {
			return ErrConflict
		}
		var nonce [16]byte
		if _, err := rand.Read(nonce[:]); err != nil {
			return ErrUnavailable
		}
		record.State = "dispatching"
		record.Attempt = hex.EncodeToString(nonce[:])
		record.StartedAt = now
		record.SMTPCode = 0
		if err := tx.Put("commands", pickupMailID(orderID), record); err != nil {
			return err
		}
		claimed = true
		return nil
	})
	if err != nil {
		return "", err
	}
	if !claimed {
		return record.State, nil
	}
	result := s.pickupMail.sender.send(ctx, pickupMailID(orderID), payload)
	// Cancellation of network work must not discard its durable outcome. This
	// bounded persistence window is included in the host worker's shutdown join.
	save, stop := context.WithTimeout(context.WithoutCancel(ctx), 3*time.Second)
	defer stop()
	err = s.repo.Within(save, shop, func(tx Tx) error {
		var current pickupMailRecord
		if err := tx.Get("commands", pickupMailID(orderID), &current); err != nil {
			return err
		}
		if current.Ciphertext != record.Ciphertext || current.Attempt != record.Attempt || current.State != "dispatching" {
			return ErrConflict
		}
		current.State = result.state
		current.SMTPCode = result.code
		if result.state == "retry_wait" {
			current.NextAttemptAt = time.Now().Add(time.Minute).Unix()
		}
		return tx.Put("commands", pickupMailID(orderID), current)
	})
	if err != nil {
		return "dispatching", ErrUnavailable
	}
	return result.state, nil
}

type PickupMailCursor struct{ ShopID, ID string }
type PickupMailJob struct {
	Cursor  PickupMailCursor
	OrderID string
}
type pickupMailIndex interface {
	PendingPickupMail(context.Context, PickupMailCursor, int) ([]PickupMailJob, error)
}

// RecoverPickupMail scans bounded batches; all SMTP ownership is in PG, not mu.
func (s *Service) RecoverPickupMail(ctx context.Context, limit int) error {
	if !Page(limit) {
		return ErrInvalid
	}
	if s.pickupMail == nil {
		return ErrUnavailable
	}
	index, ok := s.repo.(pickupMailIndex)
	if !ok {
		return ErrUnavailable
	}
	config := s.pickupMail
	config.mu.Lock()
	defer config.mu.Unlock()
	jobs, err := index.PendingPickupMail(ctx, config.after, limit)
	if err != nil {
		return err
	}
	if len(jobs) == 0 {
		config.after = PickupMailCursor{}
		return nil
	}
	var failure error
	for _, job := range jobs {
		if _, err := s.DispatchPickupMail(ctx, job.Cursor.ShopID, job.OrderID); err != nil && !errors.Is(err, ErrConflict) {
			failure = ErrUnavailable
		}
		config.after = job.Cursor
	}
	return failure
}

func (s *Service) PickupMailEnabled() bool { return s.pickupMail != nil }
