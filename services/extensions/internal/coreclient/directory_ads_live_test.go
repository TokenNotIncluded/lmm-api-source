package coreclient

import (
	"context"
	"encoding/json"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"os"
	"testing"
	"time"
)

// The Rust fixture supplies a real confirmed payment and refund, never mocks.
func TestDirectoryAdsLive(t *testing.T) {
	socket := os.Getenv("LMM_AD_TEST_SOCKET")
	if socket == "" {
		t.Skip("requires Rust-owned advertising fixture")
	}
	client, err := New(socket, []byte(os.Getenv("LMM_AD_TEST_SERVICE")))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	var pay pb.PayDirectoryAdRequest
	var refund pb.RefundDirectoryAdRequest
	if err := json.Unmarshal([]byte(os.Getenv("LMM_AD_TEST_PAYMENT")), &pay); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(os.Getenv("LMM_AD_TEST_REFUND")), &refund); err != nil {
		t.Fatal(err)
	}
	user := os.Getenv("LMM_AD_TEST_USER")
	paid, err := client.PayDirectoryAd(ctx, user, &pay)
	if err != nil || paid.GetOutcome() != "succeeded" || paid.GetChargedQuota() != 500000 {
		t.Fatalf("payment replay: %v %v", paid, err)
	}
	queried, err := client.QueryDirectoryAdPayment(ctx, user, &pay)
	if err != nil || queried.GetReceipt().GetPaymentId() != paid.GetPaymentId() {
		t.Fatalf("payment query: %v %v", queried, err)
	}
	recovered, err := client.ReconcileDirectoryAdPayment(ctx, &pay)
	if err != nil || recovered.GetReceipt().GetPaymentId() != paid.GetPaymentId() {
		t.Fatalf("payment reconcile: %v %v", recovered, err)
	}
	refunded, err := client.RefundDirectoryAd(ctx, user, &refund)
	if err != nil || refunded.GetOutcome() != "succeeded" || refunded.GetRefundedQuota() != 500000 {
		t.Fatalf("refund replay: %v %v", refunded, err)
	}
	rq, err := client.QueryDirectoryAdRefund(ctx, user, &refund)
	if err != nil || rq.GetReceipt().GetRefundId() != refunded.GetRefundId() {
		t.Fatalf("refund query: %v %v", rq, err)
	}
	rr, err := client.ReconcileDirectoryAdRefund(ctx, &refund)
	if err != nil || rr.GetReceipt().GetRefundId() != refunded.GetRefundId() {
		t.Fatalf("refund reconcile: %v %v", rr, err)
	}
	wrong, err := New(socket, []byte("wrong-advertisement-service-token-000000"))
	if err != nil {
		t.Fatal(err)
	}
	defer wrong.Close()
	if _, err = wrong.ReconcileDirectoryAdPayment(ctx, &pay); status.Code(err) != codes.Unauthenticated {
		t.Fatalf("wrong purpose credential accepted: %v", err)
	}
}
