package coreclient

import (
	"context"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc"
)

func TestPaymentProofDeadlineAllowsIndependentInquiryAndKeepsCallerDeadline(t *testing.T) {
	client := &Client{slots: make(chan struct{}, maxInFlight)}
	invoke := func(minimum, maximum time.Duration) grpc.UnaryInvoker {
		return func(ctx context.Context, _ string, _, _ any, _ *grpc.ClientConn, _ ...grpc.CallOption) error {
			deadline, ok := ctx.Deadline()
			if remaining := time.Until(deadline); !ok || remaining < minimum || remaining > maximum {
				t.Fatalf("incorrect payment RPC deadline: %v", remaining)
			}
			return nil
		}
	}
	if err := client.boundCall(context.Background(), pb.CorePayments_AcceptProviderProof_FullMethodName, nil, nil, nil, invoke(3*time.Second, 4*time.Second)); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	if err := client.boundCall(ctx, pb.CorePayments_AcceptProviderProof_FullMethodName, nil, nil, nil, invoke(0, 100*time.Millisecond)); err != nil {
		t.Fatal(err)
	}
	if len(client.slots) != 0 {
		t.Fatal("payment proof call retained admission")
	}
}
