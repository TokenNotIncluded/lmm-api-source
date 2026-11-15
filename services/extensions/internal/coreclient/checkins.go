package coreclient

import (
	"context"
	"encoding/json"
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
	s := status.Convert(e)
	if s.Code() == codes.FailedPrecondition {
		switch s.Message() {
		case "CHECKIN_VERIFICATION_REQUIRED":
			return checkins.ErrVerificationRequired
		case "CHECKIN_PASSKEY_REQUIRED":
			return checkins.ErrPasskeyRequired
		}
	}
	if s.Code() == codes.PermissionDenied && s.Message() == "CHECKIN_VERIFICATION_FAILED" {
		return checkins.ErrVerificationFailed
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
	case codes.ResourceExhausted:
		return checkins.ErrRateLimited
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
func (l CheckinLedger) Challenge(ctx context.Context, user, key string) (checkins.Challenge, error) {
	if !validUserCredential(user) {
		return checkins.Challenge{}, checkins.ErrUnauthorized
	}
	r, e := pb.NewCoreCheckinLedgerClient(l.Client.connection).Challenge(l.Client.context(ctx, user), &pb.CheckinChallengeRequest{OperationKey: key})
	if e != nil {
		return checkins.Challenge{}, checkinError(e)
	}
	var options json.RawMessage
	if r.PasskeyOptionsJson != "" {
		options = json.RawMessage(r.PasskeyOptionsJson)
		if !json.Valid(options) || options[0] != '{' {
			return checkins.Challenge{}, checkins.ErrUnavailable
		}
	}
	return checkins.Challenge{ChallengeID: r.ChallengeId, OperationKey: r.OperationKey, Day: r.Day, ExpiresAt: r.ExpiresAt, TurnstileSiteKey: r.TurnstileSiteKey, TurnstileAction: r.TurnstileAction, TurnstileCData: r.TurnstileCdata, PasskeyRequired: r.PasskeyRequired, PasskeyOptions: options}, nil
}
func (l CheckinLedger) Authorize(ctx context.Context, user string, input checkins.ClaimRequest) (checkins.Authorization, error) {
	if !validUserCredential(user) {
		return checkins.Authorization{}, checkins.ErrUnauthorized
	}
	r, e := pb.NewCoreCheckinLedgerClient(l.Client.connection).Authorize(l.Client.context(ctx, user), &pb.CheckinChallengeAuthorize{ChallengeId: input.ChallengeID, OperationKey: input.OperationKey, TurnstileToken: input.TurnstileToken, PasskeyAssertionJson: string(input.PasskeyAssertion)})
	if e != nil {
		return checkins.Authorization{}, checkinError(e)
	}
	return checkins.Authorization{ID: r.AuthorizationId, Day: r.Day, ExpiresAt: r.ExpiresAt}, nil
}
func (l CheckinLedger) Post(ctx context.Context, user string, input checkins.Posting) (checkins.Receipt, error) {
	if !validUserCredential(user) {
		return checkins.Receipt{}, checkins.ErrUnauthorized
	}
	r, e := pb.NewCoreCheckinLedgerClient(l.Client.connection).Post(l.Client.context(ctx, user), &pb.CheckinLedgerPost{Day: input.Day, AmountUnits: input.Amount, TrustLevel: input.Level, AuthorizationId: input.AuthorizationID, OperationKey: input.OperationKey})
	if e != nil {
		return checkins.Receipt{}, checkinError(e)
	}
	return checkins.Receipt{UserID: r.UserId, AccountID: r.AccountId, Day: r.Day, AmountUnits: r.AmountUnits, JournalID: r.JournalId, Timestamp: r.Timestamp, Replayed: r.Replayed}, nil
}
