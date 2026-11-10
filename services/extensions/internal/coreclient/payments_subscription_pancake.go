package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *Client) PreparePancakePurchase(ctx context.Context, user string, r *pb.PrepareEPayPurchaseRequest) (*pb.PancakeSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).PreparePancakePurchase(c.context(ctx, user), r)
}
func (c *Client) ConfirmPancakePurchase(ctx context.Context, user, id string) (*pb.PancakeSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ConfirmPancakePurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) ReadPancakePurchase(ctx context.Context, user, id string) (*pb.PancakeSubscriptionOrder, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ReadPancakePurchase(c.context(ctx, user), &pb.EPaySubscriptionOrderRequest{OrderId: id})
}
func (c *Client) AcceptPancakeSubscriptionEvent(ctx context.Context, r *pb.AcceptPancakeSubscriptionEventRequest) (*pb.PancakeSubscriptionEvent, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).AcceptPancakeSubscriptionEvent(c.context(ctx, ""), r)
}
func (c *Client) GetPancakeSubscriptionEvent(ctx context.Context, r *pb.GetPancakeSubscriptionEventRequest) (*pb.PancakeSubscriptionEvent, error) {
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).GetPancakeSubscriptionEvent(c.context(ctx, ""), r)
}

func (c *Client) ListPancakePurchases(ctx context.Context, user string) (*pb.PancakeSubscriptionOrders, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreSubscriptionPaymentsClient(c.connection).ListPancakePurchases(c.context(ctx, user), &pb.ListPancakePurchasesRequest{})
}
