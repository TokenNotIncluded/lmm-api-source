package rustfunds

import (
	"context"
	"encoding/json"
	"errors"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"google.golang.org/grpc/metadata"
	"strings"
	"testing"
)

type historicalCoreStub struct {
	coreStub
	flag    string
	command store.LegacyRefundCommand
}

func (s *historicalCoreStub) ExecuteCommerce(ctx context.Context, _ string, r *pb.ExecuteCommerceRequest) (*pb.CommerceReceipt, error) {
	s.execute++
	s.raw = r.CommandJson
	md, _ := metadata.FromOutgoingContext(ctx)
	s.flag = strings.Join(md.Get("x-lmm-commerce-refund"), ",")
	var commands []store.LegacyRefundCommand
	if err := json.Unmarshal(r.CommandJson, &commands); err != nil {
		return nil, err
	}
	s.command = commands[0]
	return &pb.CommerceReceipt{Key: s.command.Key, CommandHash: store.ID(s.command), Reference: "legacy-store-refund:" + s.command.Key, Outcome: "succeeded"}, nil
}
func (s *historicalCoreStub) ReconcileCommerce(_ context.Context, r *pb.ExecuteCommerceRequest) (*pb.QueryCommerceResponse, error) {
	if !s.known {
		return &pb.QueryCommerceResponse{}, nil
	}
	return &pb.QueryCommerceResponse{Receipt: &pb.CommerceReceipt{Key: s.command.Key, CommandHash: store.ID(s.command), Reference: "legacy-store-refund:" + s.command.Key, Outcome: "succeeded"}}, nil
}
func TestHistoricalRefundRequiresCapabilityAndReconcileCannotExecute(t *testing.T) {
	core := &historicalCoreStub{}
	a, err := New(core, features)
	if err != nil {
		t.Fatal(err)
	}
	c := store.LegacyRefundCommand{Kind: "legacy_refund", ShopID: strings.Repeat("a", 64), OrderID: strings.Repeat("b", 64), SourceRowSHA256: strings.Repeat("c", 64), Buyer: store.Account{Kind: "personal", ID: 2}, Seller: store.Account{Kind: "personal", ID: 1}, Credits: 100, Quantity: 1}
	c.Key = store.ID("store-legacy-refund-v1", c.ShopID, c.OrderID)
	if _, err = a.ExecuteLegacyRefund(context.Background(), "seller", c); !errors.Is(err, store.ErrPaymentUnavailable) || core.execute != 0 {
		t.Fatal(err)
	}
	a, err = New(core, append(append([]string{}, features...), "commerce.refund.legacy-balance.v1"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err = a.ExecuteLegacyRefund(context.Background(), "seller", c); err != nil || core.flag != "legacy-balance-v1" {
		t.Fatal(err, core.flag)
	}
	if _, err = a.ReconcileLegacyRefund(context.Background(), c); !errors.Is(err, store.ErrPending) || core.execute != 1 {
		t.Fatal(err)
	}
	core.known = true
	if _, err = a.ReconcileLegacyRefund(context.Background(), c); err != nil || core.execute != 1 {
		t.Fatal(err)
	}
}
