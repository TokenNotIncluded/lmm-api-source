// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"errors"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type recoveryAdCore struct {
	payCalls, queryPayCalls, reconcilePayCalls                int
	refundCalls, queryRefundCalls, reconcileRefundCalls       int
	payErr, queryPayErr, reconcilePayErr                      error
	refundErr, queryRefundErr, reconcileRefundErr             error
	payReceipt, queryPayReceipt, reconcilePayReceipt          *pb.DirectoryAdReceipt
	refundReceipt, queryRefundReceipt, reconcileRefundReceipt *pb.DirectoryAdReceipt
	lastPay                                                   *pb.PayDirectoryAdRequest
	lastRefund                                                *pb.RefundDirectoryAdRequest
}

func (c *recoveryAdCore) PrepareDirectoryAd(context.Context, string, *pb.PrepareDirectoryAdRequest) (*pb.DirectoryAdQuote, error) {
	return nil, errors.New("unused")
}
func (c *recoveryAdCore) PayDirectoryAd(_ context.Context, _ string, req *pb.PayDirectoryAdRequest) (*pb.DirectoryAdReceipt, error) {
	c.payCalls++
	c.lastPay = req
	return c.payReceipt, c.payErr
}
func (c *recoveryAdCore) QueryDirectoryAdPayment(context.Context, string, *pb.PayDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	c.queryPayCalls++
	if c.queryPayErr != nil {
		return nil, c.queryPayErr
	}
	return &pb.DirectoryAdResult{Receipt: c.queryPayReceipt}, nil
}
func (c *recoveryAdCore) ReconcileDirectoryAdPayment(context.Context, *pb.PayDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	c.reconcilePayCalls++
	if c.reconcilePayErr != nil {
		return nil, c.reconcilePayErr
	}
	return &pb.DirectoryAdResult{Receipt: c.reconcilePayReceipt}, nil
}
func (c *recoveryAdCore) RefundDirectoryAd(_ context.Context, _ string, req *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdReceipt, error) {
	c.refundCalls++
	c.lastRefund = req
	return c.refundReceipt, c.refundErr
}
func (c *recoveryAdCore) QueryDirectoryAdRefund(context.Context, string, *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	c.queryRefundCalls++
	if c.queryRefundErr != nil {
		return nil, c.queryRefundErr
	}
	return &pb.DirectoryAdResult{Receipt: c.queryRefundReceipt}, nil
}
func (c *recoveryAdCore) ReconcileDirectoryAdRefund(context.Context, *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdResult, error) {
	c.reconcileRefundCalls++
	if c.reconcileRefundErr != nil {
		return nil, c.reconcileRefundErr
	}
	return &pb.DirectoryAdResult{Receipt: c.reconcileRefundReceipt}, nil
}

func paidReceipt(id string) *pb.DirectoryAdReceipt {
	return &pb.DirectoryAdReceipt{RequestId: id, Outcome: "succeeded", PaymentId: "pay", CommandSha256: "ab"}
}

func TestLostPaymentResponseReusesOriginalReceipt(t *testing.T) {
	req := &pb.PayDirectoryAdRequest{ProtocolMajor: 1, RequestId: "same-request-0001", QuoteId: "q", ExpectedQuota: 50}
	core := &recoveryAdCore{payErr: status.Error(codes.Unavailable, "lost"), queryPayReceipt: paidReceipt(req.RequestId)}
	got, err := resolveAdPayment(context.Background(), core, "user-token", req)
	if err != nil || got.GetRequestId() != req.RequestId || got.GetPaymentId() != "pay" {
		t.Fatalf("receipt: %v %v", got, err)
	}
	if core.payCalls != 1 || core.queryPayCalls != 1 || core.reconcilePayCalls != 0 || core.lastPay != req {
		t.Fatalf("calls pay=%d query=%d reconcile=%d", core.payCalls, core.queryPayCalls, core.reconcilePayCalls)
	}
}

func TestLostPaymentFallsThroughServiceReadDenial(t *testing.T) {
	req := &pb.PayDirectoryAdRequest{RequestId: "same-request-0002"}
	core := &recoveryAdCore{
		payErr:              status.Error(codes.DeadlineExceeded, "lost"),
		queryPayErr:         status.Error(codes.PermissionDenied, "advertisement service permission denied"),
		reconcilePayReceipt: paidReceipt(req.RequestId),
	}
	got, err := resolveAdPayment(context.Background(), core, "user-token", req)
	if err != nil || got.GetRequestId() != req.RequestId || core.reconcilePayCalls != 1 {
		t.Fatalf("reconcile: %v %v calls=%d", got, err, core.reconcilePayCalls)
	}
}

func TestOtherUsersPaymentIsNotReconciled(t *testing.T) {
	req := &pb.PayDirectoryAdRequest{RequestId: "same-request-0003"}
	core := &recoveryAdCore{
		payErr:              status.Error(codes.Unavailable, "lost"),
		queryPayErr:         status.Error(codes.PermissionDenied, "advertisement authority denied"),
		reconcilePayReceipt: paidReceipt("other"),
	}
	_, err := resolveAdPayment(context.Background(), core, "user-token", req)
	if err != errForbidden || core.reconcilePayCalls != 0 {
		t.Fatalf("err=%v reconcile=%d", err, core.reconcilePayCalls)
	}
}

func TestUnknownPaymentStaysPendingOnTheSameRequest(t *testing.T) {
	req := &pb.PayDirectoryAdRequest{RequestId: "same-request-0004"}
	core := &recoveryAdCore{payErr: status.Error(codes.Unknown, "lost")}
	_, err := resolveAdPayment(context.Background(), core, "user-token", req)
	if err != errAdPending || core.payCalls != 1 || core.reconcilePayCalls != 1 || core.lastPay.GetRequestId() != req.RequestId {
		t.Fatalf("err=%v pay=%d reconcile=%d", err, core.payCalls, core.reconcilePayCalls)
	}
}

func TestDefinitePaymentErrorDoesNotReadBack(t *testing.T) {
	core := &recoveryAdCore{payErr: status.Error(codes.FailedPrecondition, "AD_CONFIRMATION_REQUIRED")}
	_, err := resolveAdPayment(context.Background(), core, "user-token", &pb.PayDirectoryAdRequest{RequestId: "same-request-0005"})
	if err == nil || err == errAdPending || core.queryPayCalls != 0 {
		t.Fatalf("err=%v query=%d", err, core.queryPayCalls)
	}
}

func TestLostRefundResponseReusesOriginalReceipt(t *testing.T) {
	req := &pb.RefundDirectoryAdRequest{ProtocolMajor: 1, RequestId: "ad-refund-fixed", PaymentId: "pay"}
	core := &recoveryAdCore{
		refundErr:              status.Error(codes.Unavailable, "lost"),
		queryRefundErr:         status.Error(codes.Unavailable, "lost"),
		reconcileRefundReceipt: &pb.DirectoryAdReceipt{RequestId: req.RequestId, Outcome: "succeeded", PaymentId: req.PaymentId, RefundedQuota: 50},
	}
	got, err := resolveAdRefund(context.Background(), core, "user-token", req)
	if err != nil || got.GetRequestId() != req.RequestId || got.GetRefundedQuota() != 50 || core.refundCalls != 1 || core.lastRefund != req {
		t.Fatalf("refund: %v %v calls=%d", got, err, core.refundCalls)
	}
}
