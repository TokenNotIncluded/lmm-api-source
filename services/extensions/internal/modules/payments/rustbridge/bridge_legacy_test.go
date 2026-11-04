package rustbridge

import (
	"context"
	"crypto/sha256"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	p "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

type legacyServer struct {
	pb.UnimplementedCorePaymentsServer
	t *testing.T
	c p.LegacyCallback
}

func (s *legacyServer) credential(ctx context.Context) {
	md, _ := metadata.FromIncomingContext(ctx)
	if got := md.Get("authorization"); len(got) != 1 || got[0] != "Bearer "+strings.Repeat("s", 32) || len(md.Get("x-lmm-user-credential")) != 0 {
		s.t.Error("legacy credential context leaked user authority", md)
	}
}
func (s *legacyServer) proof() *pb.LegacyProviderProofReceipt {
	h := sha256.Sum256(s.c.Payload)
	return &pb.LegacyProviderProofReceipt{ProofId: "proof", BasisId: "basis", PaymentChannelId: 4, OrderReference: s.c.OrderReference, ProviderEventId: s.c.EventID, ProviderOrderId: "ORD", PayloadSha256: h[:], AcceptedUnixSeconds: 1}
}
func (s *legacyServer) GetLegacyProviderProof(ctx context.Context, r *pb.GetLegacyProviderProofRequest) (*pb.LegacyProviderProofReceipt, error) {
	s.credential(ctx)
	h := sha256.Sum256(s.c.Payload)
	if r.PaymentChannelId != 4 || r.OrderReference != s.c.OrderReference || r.ProviderEventId != s.c.EventID || string(r.PayloadSha256) != string(h[:]) {
		s.t.Error("lookup selector changed", r)
	}
	return s.proof(), nil
}
func (s *legacyServer) AcceptLegacyProviderProof(ctx context.Context, r *pb.AcceptLegacyProviderProofRequest) (*pb.LegacyProviderProofReceipt, error) {
	s.credential(ctx)
	if r.PaymentChannelId != 4 || r.OrderReference != s.c.OrderReference || string(r.SignedPayload) != string(s.c.Payload) || string(r.Signature) != string(s.c.Signature) {
		s.t.Error("accept original evidence changed", r)
	}
	return s.proof(), nil
}
func (s *legacyServer) CreditLegacyTopup(ctx context.Context, r *pb.CreditLegacyTopupRequest) (*pb.LegacyPaymentReceipt, error) {
	s.credential(ctx)
	if r.ProofId != "proof" || r.GetMutation().GetIdempotencyKey() != "legacy-credit:basis" {
		s.t.Error("credit changed", r)
	}
	return &pb.LegacyPaymentReceipt{OperationId: "op", BasisId: "basis", ProofId: "prior_proof", State: pb.PaymentReceiptState_PAYMENT_RECEIPT_STATE_CREDIT_APPLIED, JournalId: 7}, nil
}
func TestLegacyControlledRPC(t *testing.T) {
	dir, err := os.MkdirTemp("", "legacy-rpc-")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(dir)
	socket := filepath.Join(dir, "core.sock")
	l, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	c := p.LegacyCallback{ChannelID: 4, OrderReference: "old_ref", EventID: "event", OrderID: "ORD", Payload: []byte(`{"amount":"0.123456"}`), Signature: []byte(" original sig ")}
	server := grpc.NewServer()
	pb.RegisterCorePaymentsServer(server, &legacyServer{t: t, c: c})
	go func() { _ = server.Serve(l) }()
	defer server.Stop()
	client, err := coreclient.New(socket, []byte(strings.Repeat("s", 32)))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	b, _ := New(client)
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "attacker", "x-lmm-user-credential", "user"))
	for _, fn := range []func(context.Context, p.LegacyCallback) (p.LegacyProof, error){b.LookupLegacyProof, b.AcceptLegacyProof} {
		proof, err := fn(ctx, c)
		if err != nil || proof.ID != "proof" || proof.BasisID != "basis" || proof.OrderID != "ORD" || proof.PaymentID != "" {
			t.Fatal(proof, err)
		}
	}
	receipt, err := b.CreditLegacy(ctx, "proof", "legacy-credit:basis")
	if err != nil || receipt.State != p.CreditApplied || receipt.JournalID != 7 || receipt.ProofID != "prior_proof" {
		t.Fatal(receipt, err)
	}
}
