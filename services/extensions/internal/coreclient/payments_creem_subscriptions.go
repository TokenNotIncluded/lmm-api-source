package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *Client) PrepareCreemPurchase(ctx context.Context, user string, r *pb.PrepareEPayPurchaseRequest) (*pb.CreemSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).PrepareCreemPurchase(c.context(ctx, user), r)
}
func (c *Client) ConfirmCreemPurchase(ctx context.Context, user, id string) (*pb.CreemSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ConfirmCreemPurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) ReadCreemPurchase(ctx context.Context, user, id string) (*pb.CreemSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ReadCreemPurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) AcceptCreemSubscriptionEvent(ctx context.Context, r *pb.AcceptCreemSubscriptionEventRequest) (*pb.CreemSubscriptionEvent, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).AcceptCreemSubscriptionEvent(c.context(ctx, ""), r)
}
func (c *Client) GetCreemSubscriptionEvent(ctx context.Context, r *pb.GetCreemSubscriptionEventRequest) (*pb.CreemSubscriptionEvent, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).GetCreemSubscriptionEvent(c.context(ctx, ""), r)
}

func (c *Client) ResolveCreemSubscriptionOrder(ctx context.Context, r *pb.CreemSubscriptionLookup) (*pb.EPaySubscriptionOrderRequest, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ResolveCreemSubscriptionOrder(c.context(ctx, ""), r)
}
