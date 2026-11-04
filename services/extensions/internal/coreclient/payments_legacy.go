package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

func (c *Client) AcceptLegacyProviderProof(ctx context.Context, r *pb.AcceptLegacyProviderProofRequest) (*pb.LegacyProviderProofReceipt, error) {
	return pb.NewCorePaymentsClient(c.connection).AcceptLegacyProviderProof(c.context(ctx, ""), r)
}
func (c *Client) GetLegacyProviderProof(ctx context.Context, r *pb.GetLegacyProviderProofRequest) (*pb.LegacyProviderProofReceipt, error) {
	return pb.NewCorePaymentsClient(c.connection).GetLegacyProviderProof(c.context(ctx, ""), r)
}
func (c *Client) CreditLegacyTopup(ctx context.Context, r *pb.CreditLegacyTopupRequest) (*pb.LegacyPaymentReceipt, error) {
	return pb.NewCorePaymentsClient(c.connection).CreditLegacyTopup(c.context(ctx, ""), r)
}
