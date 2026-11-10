package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *Client) PrepareEPayPurchase(ctx context.Context, user string, r *pb.PrepareEPayPurchaseRequest) (*pb.EPaySubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).PrepareEPayPurchase(c.context(ctx, user), r)
}
func (c *Client) ConfirmEPayPurchase(ctx context.Context, user, id string) (*pb.EPayCheckout, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ConfirmEPayPurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) ReadEPayPurchase(ctx context.Context, user, id string) (*pb.EPaySubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ReadEPayPurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) CancelEPayPurchase(ctx context.Context, user, id string) (*pb.EPaySubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).CancelEPayPurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) AcceptEPaySubscriptionProof(ctx context.Context, r *pb.AcceptEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).AcceptEPaySubscriptionProof(c.context(ctx, ""), r)
}
func (c *Client) GetEPaySubscriptionProof(ctx context.Context, r *pb.GetEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).GetEPaySubscriptionProof(c.context(ctx, ""), r)
}
func (c *Client) ApplyEPaySubscriptionGrant(ctx context.Context, r *pb.ApplyEPaySubscriptionGrantRequest) (*pb.EPaySubscriptionGrant, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ApplyEPaySubscriptionGrant(c.context(ctx, ""), r)
}
