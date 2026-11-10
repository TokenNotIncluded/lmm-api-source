package payments

import (
	"context"
	"database/sql"
	"errors"
	"google.golang.org/grpc/status"
)

func (p *Postgres) PutSubscriptionEvidence(ctx context.Context, e Event) (SubscriptionEvidence, error) {
	ctx, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	if e.Type != Paid || !subscriptionOrderID.MatchString(e.OrderID) || e.Evidence.EventID == "" || len(e.Evidence.Payload) == 0 || len(e.Evidence.Payload) > MaxEvidence || len(e.Evidence.Signature) == 0 || len(e.Evidence.Signature) > 2048 {
		return SubscriptionEvidence{}, ErrInvalid
	}
	digest := eventDigest(e)
	id := stableKey("subscription-candidate", e.Evidence.Provider, e.Evidence.Merchant, e.Evidence.Environment, e.OrderID, e.Evidence.EventID, eventHash(e.Evidence.Payload), eventHash(e.Evidence.Signature))
	_, err := p.db.ExecContext(ctx, `INSERT INTO payment_subscription_inbox(evidence_id,order_id,event_id,event_digest,payload,signature) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING`, id, e.OrderID, e.Evidence.EventID, digest, e.Evidence.Payload, e.Evidence.Signature)
	if err != nil {
		return SubscriptionEvidence{}, storageError(err)
	}
	var entry SubscriptionEvidence
	err = p.db.QueryRowContext(ctx, `SELECT evidence_id,order_id,event_id,event_digest,payload,signature FROM payment_subscription_inbox WHERE evidence_id=$1`, id).Scan(&entry.ID, &entry.OrderID, &entry.EventID, &entry.Digest, &entry.Payload, &entry.Signature)
	if err != nil {
		return SubscriptionEvidence{}, storageError(err)
	}
	if entry.Digest != digest {
		return SubscriptionEvidence{}, ErrConflict
	}
	return entry, nil
}
func (p *Postgres) PendingSubscriptionEvidence(ctx context.Context, limit int) ([]SubscriptionEvidence, error) {
	ctx, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	if limit < 1 || limit > 100 {
		return nil, ErrInvalid
	}
	rows, err := p.db.QueryContext(ctx, `SELECT evidence_id,order_id,event_id,event_digest,payload,signature FROM payment_subscription_inbox WHERE state='pending' AND next_attempt<=clock_timestamp() ORDER BY next_attempt,evidence_id LIMIT $1`, limit)
	if err != nil {
		return nil, storageError(err)
	}
	defer rows.Close()
	result := []SubscriptionEvidence{}
	for rows.Next() {
		var e SubscriptionEvidence
		if err = rows.Scan(&e.ID, &e.OrderID, &e.EventID, &e.Digest, &e.Payload, &e.Signature); err != nil {
			return nil, storageError(err)
		}
		result = append(result, e)
	}
	if err = rows.Err(); err != nil {
		return nil, storageError(err)
	}
	return result, nil
}
func (p *Postgres) CompleteSubscriptionEvidence(ctx context.Context, e SubscriptionEvidence, proof string, sub int64) error {
	ctx, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	if proof == "" || sub <= 0 {
		return ErrInvalid
	}
	var stored string
	err := p.db.QueryRowContext(ctx, `UPDATE payment_subscription_inbox SET state='granted',proof_id=$2,subscription_id=$3,error_code='' WHERE evidence_id=$1 AND event_digest=$4 AND payload=$5 AND (proof_id IS NULL OR proof_id=$2) AND (subscription_id IS NULL OR subscription_id=$3) RETURNING proof_id`, e.ID, proof, sub, e.Digest, e.Payload).Scan(&stored)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrConflict
	}
	if err != nil {
		return storageError(err)
	}
	return nil
}

// Rejections remain durable evidence but cannot block newer valid callbacks.
// Transient/unknown outcomes retain the exact candidate and original operation.
func (p *Postgres) RetrySubscriptionEvidence(ctx context.Context, e SubscriptionEvidence, cause error) error {
	ctx, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	// Only ingress' explicit invalid-original-proof classification is terminal.
	// General auth/config/not-found/request errors at lookup or grant remain pending.
	terminal := errors.Is(cause, ErrEvidence)
	code := status.Code(cause)
	state := "pending"
	if terminal {
		state = "rejected"
	}
	errorCode := errorCode(cause)
	if _, ok := status.FromError(cause); ok {
		errorCode = "core_" + code.String()
	}
	_, err := p.db.ExecContext(ctx, `UPDATE payment_subscription_inbox SET state=$2,error_code=$3,attempts=LEAST(attempts+1,1000000),next_attempt=clock_timestamp()+(LEAST(attempts,59)+1)*interval '1 second' WHERE evidence_id=$1 AND subscription_id IS NULL`, e.ID, state, errorCode)
	if err != nil {
		return storageError(err)
	}
	return nil
}
