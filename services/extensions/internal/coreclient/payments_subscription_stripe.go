package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *Client) PrepareStripePurchase(ctx context.Context, user string, r *pb.PrepareEPayPurchaseRequest) (*pb.StripeSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).PrepareStripePurchase(c.context(ctx, user), r)
}
func (c *Client) ConfirmStripePurchase(ctx context.Context, user, id string) (*pb.StripeSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ConfirmStripePurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) ReadStripePurchase(ctx context.Context, user, id string) (*pb.StripeSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ReadStripePurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) AcceptStripeSubscriptionEvent(ctx context.Context, r *pb.AcceptEPaySubscriptionProofRequest) (*pb.StripeSubscriptionEvent, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).AcceptStripeSubscriptionEvent(c.context(ctx, ""), r)
}
func (c *Client) GetStripeSubscriptionEvent(ctx context.Context, r *pb.GetEPaySubscriptionProofRequest) (*pb.StripeSubscriptionEvent, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).GetStripeSubscriptionEvent(c.context(ctx, ""), r)
}
