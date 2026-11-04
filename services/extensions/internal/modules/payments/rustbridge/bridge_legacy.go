package rustbridge

import (
	"context"
	"crypto/sha256"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

var _ p.LegacyCore = (*Bridge)(nil)

func (b *Bridge) LookupLegacyProof(ctx context.Context, c p.LegacyCallback) (p.LegacyProof, error) {
	h := sha256.Sum256(c.Payload)
	return legacyProof(b.client.GetLegacyProviderProof(ctx, &pb.GetLegacyProviderProofRequest{PaymentChannelId: c.ChannelID, OrderReference: c.OrderReference, ProviderEventId: c.EventID, PayloadSha256: h[:]}))
}
func (b *Bridge) AcceptLegacyProof(ctx context.Context, c p.LegacyCallback) (p.LegacyProof, error) {
	return legacyProof(b.client.AcceptLegacyProviderProof(ctx, &pb.AcceptLegacyProviderProofRequest{PaymentChannelId: c.ChannelID, OrderReference: c.OrderReference, SignedPayload: c.Payload, Signature: c.Signature}))
}
func legacyProof(r *pb.LegacyProviderProofReceipt, err error) (p.LegacyProof, error) {
	if err != nil {
		return p.LegacyProof{}, mapError(err)
	}
	if r == nil {
		return p.LegacyProof{}, p.ErrUnavailable
	}
	return p.LegacyProof{ID: r.ProofId, BasisID: r.BasisId, ChannelID: r.PaymentChannelId, OrderReference: r.OrderReference, EventID: r.ProviderEventId, OrderID: r.ProviderOrderId, PaymentID: r.ProviderPaymentId, PayloadSHA256: append([]byte(nil), r.PayloadSha256...), AcceptedUnixSeconds: r.AcceptedUnixSeconds}, nil
}
func (b *Bridge) CreditLegacy(ctx context.Context, proof, key string) (p.LegacyReceipt, error) {
	r, err := b.client.CreditLegacyTopup(ctx, &pb.CreditLegacyTopupRequest{ProofId: proof, Mutation: mutation(key)})
	if err != nil {
		return p.LegacyReceipt{}, mapError(err)
	}
	if r == nil {
		return p.LegacyReceipt{}, p.ErrUnavailable
	}
	state := ""
	switch r.State {
	case pb.PaymentReceiptState_PAYMENT_RECEIPT_STATE_CREDIT_APPLIED:
		state = p.CreditApplied
	case pb.PaymentReceiptState_PAYMENT_RECEIPT_STATE_REJECTED:
		state = p.Rejected
	default:
		return p.LegacyReceipt{}, p.ErrUnavailable
	}
	return p.LegacyReceipt{OperationID: r.OperationId, BasisID: r.BasisId, ProofID: r.ProofId, State: state, JournalID: r.JournalId, RejectionCode: r.RejectionCode}, nil
}
