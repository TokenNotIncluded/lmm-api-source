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
