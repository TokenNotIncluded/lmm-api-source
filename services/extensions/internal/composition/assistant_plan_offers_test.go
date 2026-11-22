package composition

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type planOffersPaymentPeer struct {
	pb.UnimplementedCorePaymentsServer
	calls atomic.Int32
	allow atomic.Bool
}

func (p *planOffersPaymentPeer) GetEPayConfiguration(ctx context.Context, _ *pb.GetEPayConfigurationRequest) (*pb.EPayConfiguration, error) {
	p.calls.Add(1)
	md, _ := metadata.FromIncomingContext(ctx)
	if values := md.Get("authorization"); len(values) != 1 || values[0] != "Bearer "+strings.Repeat("p", 32) {
		return nil, status.Error(codes.Unauthenticated, "payment credential rejected")
	}
	if values := md.Get("x-lmm-protocol"); len(values) != 1 || values[0] != "1" || len(md.Get("x-lmm-user-credential")) != 0 {
		return nil, status.Error(codes.Unauthenticated, "payment metadata rejected")
	}
	if !p.allow.Load() {
		return nil, status.Error(codes.PermissionDenied, "payment read permission absent")
	}
	return &pb.EPayConfiguration{Revision: strings.Repeat("a", 64)}, nil
}

// This is a local protocol peer, not a claim that a provider accepted payment.
// It exercises the actual Unix client and the production composition reader.
func TestPlanOffersCheckoutRequiresActualDedicatedPaymentRPC(t *testing.T) {
	dir, err := os.MkdirTemp("", "offers-payment-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	socket := filepath.Join(dir, "rpc")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	peer := &planOffersPaymentPeer{}
	peer.allow.Store(true)
	server := grpc.NewServer()
	pb.RegisterCorePaymentsServer(server, peer)
	go server.Serve(listener)
	t.Cleanup(server.Stop)
	client := func(token string) *coreclient.Client {
		t.Helper()
		c, err := coreclient.New(socket, []byte(token))
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { c.Close() })
		return c
	}
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "evil", "x-lmm-user-credential", "evil"))
	correct := assistantCheckoutCatalogue{runtime: &Runtime{PaymentClient: client(strings.Repeat("p", 32))}}
	if !correct.CheckoutCatalogue(ctx).PaymentRPCVerified {
		t.Fatal("actual payment credential not checked")
	}
	wrong := assistantCheckoutCatalogue{runtime: &Runtime{PaymentClient: client(strings.Repeat("q", 32))}}
	if wrong.CheckoutCatalogue(ctx).PaymentRPCVerified {
		t.Fatal("wrong payment credential accepted")
	}
	peer.allow.Store(false)
	if correct.CheckoutCatalogue(ctx).PaymentRPCVerified {
		t.Fatal("read permission absent but called available")
	}
	before := peer.calls.Load()
	if (assistantCheckoutCatalogue{runtime: &Runtime{}}).CheckoutCatalogue(ctx).PaymentRPCVerified || peer.calls.Load() != before {
		t.Fatal("missing payment client advertised")
	}
	if before != 3 {
		t.Fatal(before)
	}
}
