package rustbridge

import (
	"context"
	"errors"
	"net"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

type nativeCheckoutServer struct {
	pb.UnimplementedCorePaymentsServer
	t *testing.T
}

func (s *nativeCheckoutServer) PreparePersonalTopup(ctx context.Context, r *pb.PreparePersonalTopupRequest) (*pb.PaymentIntent, error) {
	md, _ := metadata.FromIncomingContext(ctx)
	if got := md.Get("x-lmm-user-credential"); len(got) != 1 || got[0] != strings.Repeat("u", 32) {
		s.t.Fatal("personal quote lost the user credential")
	}
	if r.AmountMinor != 100 || r.PaymentChannelId != 4 || r.Currency != "LDC" || r.GetMutation().GetIdempotencyKey() != "native_payment_key_0001" {
		s.t.Fatal("personal quote changed its frozen request")
	}
	return &pb.PaymentIntent{IntentId: 21, AccountId: 42, CreatedByUserId: 17, Provider: "epay", Merchant: "123", Environment: "live", Currency: "LDC", AmountMinor: r.AmountMinor, CreditUnits: 1000, CreditUnit: "credit_500k_usd", IdempotencyKey: r.GetMutation().GetIdempotencyKey(), RequestSha256: make([]byte, 32)}, nil
}

func (s *nativeCheckoutServer) GetEPayCheckout(ctx context.Context, r *pb.GetEPayCheckoutRequest) (*pb.EPayCheckout, error) {
	md, _ := metadata.FromIncomingContext(ctx)
	if got := md.Get("authorization"); len(got) != 1 || got[0] != "Bearer "+strings.Repeat("s", 32) || len(md.Get("x-lmm-user-credential")) != 0 {
		s.t.Fatal("checkout signing mixed worker and user authority")
	}
	method := "POST"
	if r.IntentId == 22 {
		method = "DELETE"
	}
	return &pb.EPayCheckout{IntentId: r.IntentId, OrderId: "top_" + strconv.FormatInt(r.IntentId, 10), CheckoutUrl: "https://credit.linux.do/epay/pay/submit.php?pid=123&sign=fixture", Method: method}, nil
}

func TestNativeQuoteAndEPayCheckoutBoundRPCInputs(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "core.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	pb.RegisterCorePaymentsServer(server, &nativeCheckoutServer{t: t})
	go func() { _ = server.Serve(listener) }()
	defer server.Stop()
	client, err := coreclient.New(socket, []byte(strings.Repeat("s", 32)))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	bridge, _ := New(client)
	request := p.PrepareRequest{Key: "native_payment_key_0001", ChannelID: 4, Currency: "LDC", AmountMinor: 100}
	intent, err := bridge.PreparePersonal(context.Background(), strings.Repeat("u", 32), request)
	if err != nil || intent.AccountID != 42 || intent.CreatedBy != 17 || intent.CreditUnits != 1000 || intent.Currency != "LDC" {
		t.Fatal("Core account or monetary quote lost", intent, err)
	}
	request.AccountID = 17
	if _, err = bridge.PreparePersonal(context.Background(), strings.Repeat("u", 32), request); !errors.Is(err, p.ErrInvalid) {
		t.Fatal("caller account entered personal quote", err)
	}
	checkout, err := bridge.EPayCheckout(context.Background(), 21)
	if err != nil || checkout.ID != "top_21" || checkout.Method != "POST" || checkout.PaymentID != "" {
		t.Fatal("Core checkout method or identity lost", checkout, err)
	}
	if _, err = bridge.EPayCheckout(context.Background(), 22); !errors.Is(err, p.ErrConflict) {
		t.Fatal("unsupported checkout method accepted", err)
	}
}
