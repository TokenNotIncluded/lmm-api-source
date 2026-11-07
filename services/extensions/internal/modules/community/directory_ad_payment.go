// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"errors"
	"fmt"
	"net/http"
	"strings"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// DirectoryAdCore is the dedicated Core funds service. Implementations must use
// a separate advertising service credential; user credentials are never stored.
type DirectoryAdCore interface {
	PrepareDirectoryAd(context.Context, string, *pb.PrepareDirectoryAdRequest) (*pb.DirectoryAdQuote, error)
	PayDirectoryAd(context.Context, string, *pb.PayDirectoryAdRequest) (*pb.DirectoryAdReceipt, error)
	QueryDirectoryAdPayment(context.Context, string, *pb.PayDirectoryAdRequest) (*pb.DirectoryAdResult, error)
	ReconcileDirectoryAdPayment(context.Context, *pb.PayDirectoryAdRequest) (*pb.DirectoryAdResult, error)
	RefundDirectoryAd(context.Context, string, *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdReceipt, error)
	QueryDirectoryAdRefund(context.Context, string, *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdResult, error)
	ReconcileDirectoryAdRefund(context.Context, *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdResult, error)
}

var errAdReceipt = errors.New("invalid or incomplete Core advertising receipt")

// A hidden placement is durable before this call. Repeating hide resumes the
// same Core refund; legacy placements without a Core receipt stay pending.
func (h *handlers) refundAd(ctx context.Context, user string, ad *DirectoryAd) error {
	if ad.RefundStatus != "pending_core" || h.s.adCore == nil {
		return nil
	}
	var payment string
	err := h.s.db.QueryRowContext(ctx, `SELECT payment_id FROM lmm_community.directory_ad_intents WHERE request_id=$1 AND payment_state='paid'`, ad.requestID).Scan(&payment)
	if errors.Is(err, sql.ErrNoRows) {
		return nil
	}
	if err != nil {
		return errUnavailable
	}
	key := fmt.Sprintf("ad-refund-%x", sha256.Sum256([]byte(ad.requestID)))
	_, err = h.s.db.ExecContext(ctx, `UPDATE lmm_community.directory_ad_intents SET refund_request_id=$2,refund_state='pending' WHERE request_id=$1 AND refund_state<>'refunded'`, ad.requestID, key)
	if err != nil {
		return errUnavailable
	}
	receipt, err := resolveAdRefund(ctx, h.s.adCore, user, &pb.RefundDirectoryAdRequest{ProtocolMajor: 1, RequestId: key, PaymentId: payment})
	if err != nil {
		return err
	}
	if receipt == nil || receipt.GetRequestId() != key || receipt.GetPaymentId() != payment || !adDigest.MatchString(receipt.GetCommandSha256()) {
		return errAdPending
	}
	if receipt.GetOutcome() == "pending" {
		return nil
	}
	if receipt.GetOutcome() != "succeeded" || receipt.GetRefundedQuota() != ad.ChargedQuota || receipt.GetRefundedAt() <= 0 || receipt.GetRefundId() == "" {
		return errAdPending
	}
	err = h.s.tx(ctx, func(ctx context.Context, tx *sql.Tx) error {
		if _, e := tx.ExecContext(ctx, `UPDATE lmm_community.directory_ads SET refund_status='refunded',refunded_at=$2 WHERE id=$1 AND status='hidden'`, ad.ID, receipt.GetRefundedAt()); e != nil {
			return e
		}
		_, e := tx.ExecContext(ctx, `UPDATE lmm_community.directory_ad_intents SET refund_state='refunded',refund_command_sha256=$2 WHERE request_id=$1`, ad.requestID, receipt.GetCommandSha256())
		return e
	})
	if err != nil {
		return errAdPending
	}
	ad.RefundStatus = "refunded"
	ad.RefundedAt = receipt.GetRefundedAt()
	return nil
}

// validateAdPaymentReceipt checks Core's response before any publication. The
// command digest is opaque; it is persisted but never computed by Go.
func validateAdPaymentReceipt(requestID string, quota int64, r *pb.DirectoryAdReceipt) error {
	if r == nil || r.GetRequestId() != requestID || !adDigest.MatchString(r.GetCommandSha256()) {
		return errAdReceipt
	}
	switch r.GetOutcome() {
	case "succeeded":
		if !adDigest.MatchString(r.GetPaymentId()) || r.GetChargedQuota() != quota || r.GetPaidAt() <= 0 || r.GetAdExpiresAt() != r.GetPaidAt()+30*24*60*60 || r.GetRejectionCode() != "" {
			return errAdReceipt
		}
	case "rejected":
		if r.GetRejectionCode() == "" {
			return errAdReceipt
		}
	default:
		return errAdReceipt
	}
	return nil
}

var errAdPending = &apiError{http.StatusServiceUnavailable, "AI_DIRECTORY_AD_PAYMENT_PENDING", "Payment outcome is pending; retry the same request"}
var errAdConflict = &apiError{http.StatusConflict, "AI_DIRECTORY_AD_REQUEST_CONFLICT", "Advertisement request conflicts with an existing request"}

func adRPCError(err error) error {
	switch status.Code(err) {
	case codes.AlreadyExists:
		return errAdConflict
	case codes.Unauthenticated:
		return &apiError{http.StatusUnauthorized, "AI_DIRECTORY_LOGIN_REQUIRED", "Sign in first"}
	case codes.PermissionDenied:
		return errForbidden
	case codes.FailedPrecondition:
		switch status.Convert(err).Message() {
		case "AD_CONFIRMATION_REQUIRED":
			return &apiError{http.StatusConflict, "AI_DIRECTORY_AD_CONFIRMATION_REQUIRED", "Confirm the payment in Core"}
		case "AD_QUOTE_EXPIRED":
			return &apiError{http.StatusConflict, "AI_DIRECTORY_AD_QUOTE_EXPIRED", "The quote expired"}
		}
	}
	return errAdPending
}

// A lost Pay/Refund response is not a new charge. Unavailable and unknown
// transport errors keep the original request; a missing receipt stays pending.
func ambiguousAdRPC(err error) bool {
	switch status.Code(err) {
	case codes.Canceled, codes.Unknown, codes.DeadlineExceeded, codes.Internal, codes.Aborted, codes.Unavailable, codes.DataLoss:
		return true
	default:
		return false
	}
}

func adServiceReadDenied(err error) bool {
	s, ok := status.FromError(err)
	return ok && s.Code() == codes.PermissionDenied && s.Message() == "advertisement service permission denied"
}

func adResultReceipt(result *pb.DirectoryAdResult, err error) (*pb.DirectoryAdReceipt, error) {
	if err != nil {
		return nil, err
	}
	if result == nil {
		return nil, nil
	}
	return result.GetReceipt(), nil
}

// observeAdReceipt reads the original command. An absent receipt is UNKNOWN and
// does not authorize another request id. Service-level read denial falls through
// to reconcile; a user-authority denial does not.
func observeAdReceipt(query, reconcile func() (*pb.DirectoryAdResult, error)) (*pb.DirectoryAdReceipt, error) {
	receipt, err := adResultReceipt(query())
	if err != nil {
		if !ambiguousAdRPC(err) && !adServiceReadDenied(err) {
			return nil, adRPCError(err)
		}
		receipt = nil
	}
	if receipt != nil {
		return receipt, nil
	}
	receipt, err = adResultReceipt(reconcile())
	if err != nil || receipt == nil {
		return nil, errAdPending
	}
	return receipt, nil
}

func resolveAdPayment(ctx context.Context, core DirectoryAdCore, user string, req *pb.PayDirectoryAdRequest) (*pb.DirectoryAdReceipt, error) {
	receipt, err := core.PayDirectoryAd(ctx, user, req)
	if err == nil {
		return receipt, nil
	}
	if !ambiguousAdRPC(err) {
		return nil, adRPCError(err)
	}
	return observeAdReceipt(
		func() (*pb.DirectoryAdResult, error) { return core.QueryDirectoryAdPayment(ctx, user, req) },
		func() (*pb.DirectoryAdResult, error) { return core.ReconcileDirectoryAdPayment(ctx, req) },
	)
}

func resolveAdRefund(ctx context.Context, core DirectoryAdCore, user string, req *pb.RefundDirectoryAdRequest) (*pb.DirectoryAdReceipt, error) {
	receipt, err := core.RefundDirectoryAd(ctx, user, req)
	if err == nil {
		return receipt, nil
	}
	if !ambiguousAdRPC(err) {
		return nil, adRPCError(err)
	}
	return observeAdReceipt(
		func() (*pb.DirectoryAdResult, error) { return core.QueryDirectoryAdRefund(ctx, user, req) },
		func() (*pb.DirectoryAdResult, error) { return core.ReconcileDirectoryAdRefund(ctx, req) },
	)
}

func (h *handlers) prepareAd(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if h.s.adCore == nil {
		respond(w, nil, errAdPaymentUnavailable)
		return
	}
	var in directoryAdInput
	if decode(w, r, 8<<10, &in, false) != nil {
		respond(w, nil, errAdInvalid)
		return
	}
	in, err = normalizeDirectoryAdInput(in)
	if err != nil || in.ExpectedQuota != 0 || in.QuoteID != "" || in.ContentSHA256 != "" {
		respond(w, nil, errAdInvalid)
		return
	}
	digest := directoryAdContentHash(in)
	_, err = h.s.db.ExecContext(r.Context(), `INSERT INTO lmm_community.directory_ad_intents(request_id,owner_user_id,name,url,summary,description,bid_cents,content_sha256) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING`, in.RequestID, actor.GetUserId(), in.Name, in.URL, in.Summary, in.Description, in.BidCents, digest)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	var owner, bid int64
	var name, url, summary, description, saved string
	err = h.s.db.QueryRowContext(r.Context(), `SELECT owner_user_id,name,url,summary,description,bid_cents,content_sha256 FROM lmm_community.directory_ad_intents WHERE request_id=$1`, in.RequestID).Scan(&owner, &name, &url, &summary, &description, &bid, &saved)
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if owner != actor.GetUserId() || name != in.Name || url != in.URL || summary != in.Summary || description != in.Description || bid != in.BidCents || saved != digest {
		respond(w, nil, errAdConflict)
		return
	}
	user := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
	q, err := h.s.adCore.PrepareDirectoryAd(r.Context(), user, &pb.PrepareDirectoryAdRequest{ProtocolMajor: 1, RequestId: in.RequestID, BidCents: in.BidCents, ContentVersion: 1, Name: in.Name, Url: in.URL, Summary: in.Summary, Description: in.Description, ContentSha256: digest})
	if err != nil {
		respond(w, nil, adRPCError(err))
		return
	}
	if q == nil || q.GetRequestId() != in.RequestID || q.GetContentSha256() != digest || q.GetBidCents() != in.BidCents || !adDigest.MatchString(q.GetQuoteId()) || q.GetQuota() <= 0 || q.GetCurrency() != "USD" || q.GetPricingSchemaVersion() != 2 || q.GetQuoteContractVersion() != 1 || q.GetDurationDays() != 30 || q.GetMinBidCents() != 100 || q.GetMaxBidCents() != 1000000 || q.GetExpiresAt() <= 0 || q.GetConfirmationPath() != "/core/v1/directory-ads/confirm/"+q.GetQuoteId() {
		respond(w, nil, errAdPending)
		return
	}
	result, err := h.s.db.ExecContext(r.Context(), `UPDATE lmm_community.directory_ad_intents SET quote_id=$2,quota=$3,quote_expires_at=$4,payment_state=CASE WHEN payment_state='preparing' THEN 'prepared' ELSE payment_state END WHERE request_id=$1 AND (quote_id='' OR (quote_id=$2 AND quota=$3 AND quote_expires_at=$4))`, in.RequestID, q.GetQuoteId(), q.GetQuota(), q.GetExpiresAt())
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	n, err := result.RowsAffected()
	if err != nil || n != 1 {
		respond(w, nil, errAdConflict)
		return
	}
	respond(w, map[string]any{"quote_id": q.GetQuoteId(), "request_id": q.GetRequestId(), "content_sha256": digest, "bid_cents": q.GetBidCents(), "quota": q.GetQuota(), "currency": q.GetCurrency(), "pricing_schema_version": q.GetPricingSchemaVersion(), "quote_contract_version": q.GetQuoteContractVersion(), "expires_at": q.GetExpiresAt(), "duration_days": q.GetDurationDays(), "min_bid_cents": q.GetMinBidCents(), "max_bid_cents": q.GetMaxBidCents(), "confirmation_path": q.GetConfirmationPath()}, nil)
}

func (h *handlers) payAd(w http.ResponseWriter, r *http.Request, userID int64, in directoryAdInput) {
	if h.s.adCore == nil {
		respond(w, nil, errAdPaymentUnavailable)
		return
	}
	normalized, err := normalizeDirectoryAdInput(in)
	if err != nil || in.ExpectedQuota <= 0 || !adDigest.MatchString(in.QuoteID) || !adDigest.MatchString(in.ContentSHA256) || directoryAdContentHash(normalized) != in.ContentSHA256 {
		respond(w, nil, errAdInvalid)
		return
	}
	var owner, bid, quota int64
	var name, url, summary, description, digest, quote, state string
	err = h.s.db.QueryRowContext(r.Context(), `SELECT owner_user_id,name,url,summary,description,bid_cents,content_sha256,quote_id,quota,payment_state FROM lmm_community.directory_ad_intents WHERE request_id=$1`, in.RequestID).Scan(&owner, &name, &url, &summary, &description, &bid, &digest, &quote, &quota, &state)
	if errors.Is(err, sql.ErrNoRows) {
		respond(w, nil, errAdConflict)
		return
	}
	if err != nil {
		respond(w, nil, errUnavailable)
		return
	}
	if owner != userID || name != normalized.Name || url != normalized.URL || summary != normalized.Summary || description != normalized.Description || bid != in.BidCents || digest != in.ContentSHA256 || quote != in.QuoteID || quota != in.ExpectedQuota {
		respond(w, nil, errAdConflict)
		return
	}
	request := &pb.PayDirectoryAdRequest{ProtocolMajor: 1, RequestId: in.RequestID, QuoteId: quote, ExpectedQuota: quota, ContentSha256: digest}
	receipt, err := resolveAdPayment(r.Context(), h.s.adCore, strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "), request)
	if err != nil {
		respond(w, nil, err)
		return
	}
	if validateAdPaymentReceipt(in.RequestID, quota, receipt) != nil || receipt.GetPaymentId() != quote {
		respond(w, nil, errAdPending)
		return
	}
	if receipt.GetOutcome() == "rejected" {
		_, err = h.s.db.ExecContext(r.Context(), `UPDATE lmm_community.directory_ad_intents SET payment_state='rejected',payment_id=$2,command_sha256=$3,rejection_code=$4 WHERE request_id=$1 AND payment_state<>'paid'`, in.RequestID, receipt.GetPaymentId(), receipt.GetCommandSha256(), receipt.GetRejectionCode())
		if err != nil {
			respond(w, nil, errUnavailable)
			return
		}
		switch receipt.GetRejectionCode() {
		case "insufficient_funds":
			respond(w, nil, &apiError{http.StatusPaymentRequired, "AI_DIRECTORY_AD_INSUFFICIENT_BALANCE", "Insufficient balance"})
		case "budget_exceeded":
			respond(w, nil, &apiError{http.StatusPaymentRequired, "AI_DIRECTORY_AD_BUDGET_EXCEEDED", "Budget exceeded"})
		default:
			respond(w, nil, errAdConflict)
		}
		return
	}
	var ad DirectoryAd
	created := false
	err = h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var current string
		if e := tx.QueryRowContext(ctx, `SELECT payment_state FROM lmm_community.directory_ad_intents WHERE request_id=$1 FOR UPDATE`, in.RequestID).Scan(&current); e != nil {
			return e
		}
		if current == "rejected" {
			return errAdConflict
		}
		e := tx.QueryRowContext(ctx, `INSERT INTO lmm_community.directory_ads(owner_user_id,name,url,summary,description,bid_cents,charged_quota,request_id,status,paid_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,$10) ON CONFLICT (request_id) DO NOTHING RETURNING id`, userID, name, url, summary, description, bid, quota, in.RequestID, receipt.GetPaidAt(), receipt.GetAdExpiresAt()).Scan(&ad.ID)
		if e == nil {
			created = true
		} else if !errors.Is(e, sql.ErrNoRows) {
			return e
		}
		ad, e = scanAd(tx.QueryRowContext(ctx, `SELECT `+adColumns+` FROM lmm_community.directory_ads WHERE request_id=$1`, in.RequestID))
		if e != nil {
			return e
		}
		if ad.OwnerUserID != userID || ad.ChargedQuota != quota || ad.PaidAt != receipt.GetPaidAt() || ad.ExpiresAt != receipt.GetAdExpiresAt() {
			return errAdConflict
		}
		_, e = tx.ExecContext(ctx, `UPDATE lmm_community.directory_ad_intents SET payment_state='paid',payment_id=$2,command_sha256=$3 WHERE request_id=$1`, in.RequestID, receipt.GetPaymentId(), receipt.GetCommandSha256())
		return e
	})
	if err != nil {
		respond(w, nil, errAdPending)
		return
	}
	respond(w, map[string]any{"ad": ad, "created": created, "charged_quota": quota}, nil)
}
