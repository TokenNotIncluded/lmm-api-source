package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

// Only the explicitly negotiated store extension survives replacement of caller
// metadata. Authorization/user credentials still come from c.context, never
// from incoming metadata or the command body. Other module calls are untouched.
func (c *Client) commerceContext(ctx context.Context, user string) context.Context {
	in, _ := metadata.FromOutgoingContext(ctx)
	flags := in.Get("x-lmm-commerce-refund")
	out := c.context(ctx, user)
	if len(flags) == 1 && (flags[0] == "partial-v1" || flags[0] == "legacy-balance-v1") {
		out = metadata.AppendToOutgoingContext(out, "x-lmm-commerce-refund", flags[0])
	}
	return out
}

// Use a separate Client initialized with only the commerce service credential.
// User metadata is independent of that credential and is replaced, not merged.
func (c *Client) ExecuteCommerce(ctx context.Context, user string, r *pb.ExecuteCommerceRequest) (*pb.CommerceReceipt, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreCommerceClient(c.connection).ExecuteCommerce(c.commerceContext(ctx, user), r)
}
func (c *Client) QueryCommerce(ctx context.Context, user string, r *pb.QueryCommerceRequest) (*pb.QueryCommerceResponse, error) {
	if !validUserCredential(user) {
		return nil, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	return pb.NewCoreCommerceClient(c.connection).QueryCommerce(c.context(ctx, user), r)
}

// Completion consumes a previously user-authorized fixed capture grant. It
// cannot select a new payer, amount or recipient and never retains a user token.
func (c *Client) CompleteCommerceDelivery(ctx context.Context, r *pb.CompleteDeliveryRequest) (*pb.CommerceReceipt, error) {
	return pb.NewCoreCommerceClient(c.connection).CompleteDelivery(c.context(ctx, ""), r)
}

// Reconcile reads only the receipt of an already persisted command. The core
// binds this service identity to the original operation; UNKNOWN never executes.
func (c *Client) ReconcileCommerce(ctx context.Context, r *pb.ExecuteCommerceRequest) (*pb.QueryCommerceResponse, error) {
	return pb.NewCoreCommerceClient(c.connection).ReconcileCommerce(c.commerceContext(ctx, ""), r)
}
