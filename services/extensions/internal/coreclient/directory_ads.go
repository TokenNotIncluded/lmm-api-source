// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package coreclient

import (
	"context"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

func (c *Client) PrepareDirectoryAd(ctx context.Context, user string, req *pb.PrepareDirectoryAdRequest) (*pb.DirectoryAdQuote, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).PrepareDirectoryAd(c.context(ctx, user), req)
}
func (c *Client) PayDirectoryAd(ctx context.Context, user string, req *pb.PayDirectoryAdRequest) (*pb.DirectoryAdReceipt, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).PayDirectoryAd(c.context(ctx, user), req)
}
func (c *Client) QueryDirectoryAdPayment(ctx context.Context, user string, req *pb.PayDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).QueryDirectoryAdPayment(c.context(ctx, user), req)
}
func (c *Client) ReconcileDirectoryAdPayment(ctx context.Context, req *pb.PayDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).ReconcileDirectoryAdPayment(c.context(ctx, ""), req)
}
func (c *Client) RefundDirectoryAd(ctx context.Context, user string, req *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdReceipt, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).RefundDirectoryAd(c.context(ctx, user), req)
}
func (c *Client) QueryDirectoryAdRefund(ctx context.Context, user string, req *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).QueryDirectoryAdRefund(c.context(ctx, user), req)
}
func (c *Client) ReconcileDirectoryAdRefund(ctx context.Context, req *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	return pb.NewCoreDirectoryAdsClient(c.connection).ReconcileDirectoryAdRefund(c.context(ctx, ""), req)
}
