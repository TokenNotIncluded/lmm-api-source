package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// CheckinLedger must wrap a client with the separate funding credential, never
// the identity transport credential. Core independently checks its money ceiling.
type CheckinLedger struct{ Client *Client }

func checkinError(e error) error {
	if e == nil {
		return nil
	}
	switch status.Code(e) {
	case codes.InvalidArgument:
		return checkins.ErrInvalid
	case codes.Unauthenticated:
		return checkins.ErrUnauthorized
	case codes.PermissionDenied:
		return checkins.ErrForbidden
	case codes.AlreadyExists, codes.FailedPrecondition:
		return checkins.ErrConflict
	default:
		return checkins.ErrUnavailable
	}
}
func (l CheckinLedger) Inspect(ctx context.Context, user string) (checkins.Authority, error) {
	if !validUserCredential(user) {
		return checkins.Authority{}, checkins.ErrUnauthorized
	}
	r, e := pb.NewCoreCheckinLedgerClient(l.Client.connection).Inspect(l.Client.context(ctx, user), &pb.CheckinLedgerInspect{})
	if e != nil {
		return checkins.Authority{}, checkinError(e)
	}
	return checkins.Authority{UserID: r.UserId, AccountID: r.AccountId, Day: r.Day, Timestamp: r.Timestamp, TrustLevel: r.TrustLevel, MaxDailyUnits: r.MaxDailyUnits, Blocked: r.Blocked}, nil
}
func (l CheckinLedger) Post(ctx context.Context, user, day string, amount int64, level uint32) (checkins.Receipt, error) {
	if !validUserCredential(user) {
		return checkins.Receipt{}, checkins.ErrUnauthorized
	}
	r, e := pb.NewCoreCheckinLedgerClient(l.Client.connection).Post(l.Client.context(ctx, user), &pb.CheckinLedgerPost{Day: day, AmountUnits: amount, TrustLevel: level})
	if e != nil {
		return checkins.Receipt{}, checkinError(e)
	}
	return checkins.Receipt{UserID: r.UserId, AccountID: r.AccountId, Day: r.Day, AmountUnits: r.AmountUnits, JournalID: r.JournalId, Timestamp: r.Timestamp, Replayed: r.Replayed}, nil
}
