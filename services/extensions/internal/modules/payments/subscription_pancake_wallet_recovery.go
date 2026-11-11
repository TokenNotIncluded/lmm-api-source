package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"net/http"
)

// Original wallet proofs use the wallet settlement path. This queue only closes
// the crash gap before Core proof admission and never grants subscriptions.
func (s *PancakeSubscriptions) AttachWallet(wallet *Service) {
	s.wallet = wallet
	wallet.pancakeRecovery = s
}
func (s *PancakeSubscriptions) receiveWallet(ctx context.Context, wallet *Service, channel int64, r *http.Request) error {
	raw, sig, err := callbackBytes(r, "X-Waffo-Signature")
	if err != nil {
		return err
	}
	id := fmt.Sprintf("%x", sha256.Sum256(append([]byte(fmt.Sprintf("pancake-wallet-v1:%d:%x:", channel, sha256.Sum256(raw))), sig...)))
	if _, err = s.exec(ctx, `INSERT INTO payment_pancake_wallet_inbox(evidence_id,channel_id,payload,signature) VALUES($1,$2,$3,$4) ON CONFLICT(evidence_id) DO NOTHING`, id, channel, raw, sig); err != nil {
		return storageError(err)
	}
	var state string
	if err = s.scan(ctx, `SELECT state FROM payment_pancake_wallet_inbox WHERE evidence_id=$1`, []any{id}, &state); err != nil {
		return storageError(err)
	}
	if state == "accepted" {
		return nil
	}
	if state == "rejected" {
		return ErrEvidence
	}
	return s.settleWallet(ctx, wallet, id, channel, raw, sig)
}
func (s *PancakeSubscriptions) settleWallet(ctx context.Context, wallet *Service, id string, channel int64, raw, sig []byte) error {
	r, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://original-evidence.invalid/callbacks", bytes.NewReader(raw))
	if err != nil {
		return ErrInvalid
	}
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("X-Waffo-Signature", string(sig))
	err = wallet.receiveOriginal(ctx, channel, r)
	state := "pending"
	code := "retry_original"
	// A typed nil []byte is encoded as empty bytea by lib/pq, not SQL NULL.
	var result any
	if err == nil {
		state = "accepted"
		code = ""
		result = []byte("durable_original_wallet_event")
	} else if errors.Is(err, ErrInvalid) || errors.Is(err, ErrEvidence) {
		state = "rejected"
		code = "invalid_original"
	}
	_, save := s.exec(ctx, `UPDATE payment_pancake_wallet_inbox SET state=$2,result=$3,error_code=$4,attempts=least(attempts+1,1000000),next_attempt=clock_timestamp()+make_interval(secs=>least(3600,1+power(2,least(attempts,10))::integer)) WHERE evidence_id=$1 AND state='pending'`, id, state, result, code)
	if save != nil {
		return storageError(save)
	}
	return err
}
func (s *PancakeSubscriptions) recoverWallet(ctx context.Context, limit int) error {
	if s.wallet == nil {
		return nil
	}
	bounded, cancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(bounded, `SELECT evidence_id,channel_id,payload,signature FROM payment_pancake_wallet_inbox WHERE state='pending' AND next_attempt<=clock_timestamp() ORDER BY next_attempt,evidence_id LIMIT $1`, limit)
	if err != nil {
		cancel()
		return storageError(err)
	}
	type item struct {
		id       string
		channel  int64
		raw, sig []byte
	}
	var items []item
	for rows.Next() {
		var i item
		if err = rows.Scan(&i.id, &i.channel, &i.raw, &i.sig); err != nil {
			break
		}
		items = append(items, i)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	cancel()
	if err != nil {
		return storageError(err)
	}
	var failures []error
	for _, i := range items {
		if e := s.settleWallet(ctx, s.wallet, i.id, i.channel, i.raw, i.sig); e != nil {
			failures = append(failures, e)
		}
	}
	return errors.Join(failures...)
}
