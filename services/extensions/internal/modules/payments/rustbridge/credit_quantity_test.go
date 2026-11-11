package rustbridge

import (
	"bytes"
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"google.golang.org/grpc"
	"google.golang.org/protobuf/proto"
	"net"
	"path/filepath"
	"strings"
	"testing"
)

type quantityRPCServer struct {
	pb.UnimplementedCorePaymentsServer
	t     *testing.T
	calls int
}

func (s *quantityRPCServer) quote(key string, qty, amount int64) *pb.PaymentIntent {
	s.calls++
	if qty != 1000000 || amount != 0 {
		s.t.Error("wire quantity/zero charge changed", qty, amount)
	}
	return &pb.PaymentIntent{IntentId: 31, AccountId: 42, CreatedByUserId: 17, Provider: "waffo_pancake", Merchant: "synthetic", Environment: "test", Currency: "CNY", AmountMinor: 1327, CreditUnits: qty, CreditUnit: "credit_500k_usd", IdempotencyKey: key, RequestSha256: bytes.Repeat([]byte{7}, 32)}
}
func (s *quantityRPCServer) PreparePersonalTopup(_ context.Context, r *pb.PreparePersonalTopupRequest) (*pb.PaymentIntent, error) {
	return s.quote(r.GetMutation().GetIdempotencyKey(), r.RequestedCreditUnits, r.AmountMinor), nil
}
func (s *quantityRPCServer) PrepareTopup(_ context.Context, r *pb.PrepareTopupRequest) (*pb.PaymentIntent, error) {
	if r.AccountId != 42 {
		s.t.Error("explicit account lost")
	}
	return s.quote(r.GetMutation().GetIdempotencyKey(), r.RequestedCreditUnits, r.AmountMinor), nil
}
func TestBridgeCreditQuantityWireAndCanonicalQuote(t *testing.T) {
	listener, err := net.Listen("unix", filepath.Join(t.TempDir(), "core.sock"))
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	fixture := &quantityRPCServer{t: t}
	pb.RegisterCorePaymentsServer(server, fixture)
	go server.Serve(listener)
	defer server.Stop()
	client, err := coreclient.New(listener.Addr().String(), []byte(strings.Repeat("s", 32)))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	bridge, _ := New(client)
	req := p.PrepareRequest{Key: "quantity_rpc_fixture_01", ChannelID: 3, Currency: "CNY", RequestedCreditUnits: 1000000}
	for _, personal := range []bool{true, false} {
		var intent p.Intent
		if personal {
			intent, err = bridge.PreparePersonal(context.Background(), strings.Repeat("u", 32), req)
		} else {
			req.AccountID = 42
			intent, err = bridge.Prepare(context.Background(), strings.Repeat("u", 32), req)
		}
		if err != nil || intent.AmountMinor != 1327 || intent.CreditUnits != 1000000 || intent.RequestedCreditUnits != 1000000 || !bytes.Equal(intent.RequestHash, bytes.Repeat([]byte{7}, 32)) {
			t.Fatal(intent, err)
		}
	}
	if fixture.calls != 2 {
		t.Fatal("RPC path missing")
	}
	// Appended proto3 defaults leave old request bytes unchanged.
	old := &pb.PrepareTopupRequest{AccountId: 42, PaymentChannelId: 3, Currency: "USD", AmountMinor: 100}
	before, _ := proto.Marshal(old)
	old.RequestedCreditUnits = 0
	after, _ := proto.Marshal(old)
	if !bytes.Equal(before, after) {
		t.Fatal("default changed wire bytes")
	}
	if old.ProtoReflect().Descriptor().Fields().ByName("requested_credit_units").Number() != 6 || (&pb.PreparePersonalTopupRequest{}).ProtoReflect().Descriptor().Fields().ByName("requested_credit_units").Number() != 5 {
		t.Fatal("wrong field numbers")
	}
}
