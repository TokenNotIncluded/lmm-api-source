// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package rustfunds

import (
	"context"
	"encoding/json"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

func (a *Adapter) LegacyRefundAvailable() bool {
	return a.Available() && a.legacyRefund && a.reconciliation
}
func (a *Adapter) legacyCommand(c store.LegacyRefundCommand) ([]byte, error) {
	if !a.LegacyRefundAvailable() {
		return nil, store.ErrPaymentUnavailable
	}
	if !c.Valid() {
		return nil, store.ErrInvalid
	}
	return json.Marshal([]any{c})
}
func legacyReceipt(r *pb.CommerceReceipt, c store.LegacyRefundCommand) (store.Receipt, error) {
	if r == nil || r.GetKey() != c.Key || r.GetCommandHash() != store.ID(c) || r.GetReference() != "legacy-store-refund:"+c.Key || r.GetOutcome() != "succeeded" {
		return store.Receipt{}, store.ErrPending
	}
	return store.Receipt{Key: r.GetKey(), CommandHash: r.GetCommandHash(), Reference: r.GetReference(), Outcome: r.GetOutcome()}, nil
}
func (a *Adapter) ExecuteLegacyRefund(ctx context.Context, user string, c store.LegacyRefundCommand) (store.Receipt, error) {
	raw, err := a.legacyCommand(c)
	if err != nil {
		return store.Receipt{}, err
	}
	ctx = metadata.AppendToOutgoingContext(ctx, "x-lmm-commerce-refund", "legacy-balance-v1")
	r, err := a.core.ExecuteCommerce(ctx, user, &pb.ExecuteCommerceRequest{ProtocolMajor: 1, CommandJson: raw})
	if err != nil {
		switch status.Code(err) {
		case codes.Unauthenticated:
			return store.Receipt{}, store.ErrUnauthorized
		case codes.PermissionDenied:
			return store.Receipt{}, store.ErrForbidden
		case codes.AlreadyExists:
			return store.Receipt{}, store.ErrConflict
		case codes.NotFound:
			return store.Receipt{}, store.ErrPaymentUnavailable
		}
		return store.Receipt{}, store.ErrPending
	}
	return legacyReceipt(r, c)
}
func (a *Adapter) ReconcileLegacyRefund(ctx context.Context, c store.LegacyRefundCommand) (store.Receipt, error) {
	raw, err := a.legacyCommand(c)
	if err != nil {
		return store.Receipt{}, err
	}
	ctx = metadata.AppendToOutgoingContext(ctx, "x-lmm-commerce-refund", "legacy-balance-v1")
	r, err := a.core.ReconcileCommerce(ctx, &pb.ExecuteCommerceRequest{ProtocolMajor: 1, CommandJson: raw})
	if err != nil || r == nil || r.GetReceipt() == nil {
		return store.Receipt{}, store.ErrPending
	}
	return legacyReceipt(r.GetReceipt(), c)
}
