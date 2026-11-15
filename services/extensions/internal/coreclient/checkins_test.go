package coreclient

import (
	"context"
	"encoding/json"
	"net"
	"strings"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/grpc/test/bufconn"
)

type checkinProofServer struct {
	pb.UnimplementedCoreCheckinLedgerServer
	t       *testing.T
	key     string
	user    string
	service string
}

func (s *checkinProofServer) identity(ctx context.Context) {
	s.t.Helper()
	m, _ := metadata.FromIncomingContext(ctx)
	for name, want := range map[string]string{"authorization": "Bearer " + s.service, "x-lmm-user-credential": s.user, "x-lmm-protocol": "1"} {
		if values := m.Get(name); len(values) != 1 || values[0] != want {
			s.t.Errorf("invalid %s metadata", name)
		}
	}
}

func (s *checkinProofServer) Challenge(ctx context.Context, input *pb.CheckinChallengeRequest) (*pb.CheckinChallenge, error) {
	s.identity(ctx)
	if input.OperationKey != s.key {
		s.t.Error("challenge lost operation binding")
	}
	return &pb.CheckinChallenge{ChallengeId: "wire-challenge", OperationKey: s.key, Day: "2026-11-15", ExpiresAt: 1794715320, TurnstileSiteKey: "wire-site", TurnstileAction: "daily_checkin", TurnstileCdata: "wire-context", PasskeyRequired: true, PasskeyOptionsJson: `{"publicKey":{"challenge":"value"}}`}, nil
}

func (s *checkinProofServer) Authorize(ctx context.Context, input *pb.CheckinChallengeAuthorize) (*pb.CheckinAuthorization, error) {
	s.identity(ctx)
	if input.OperationKey != s.key || input.ChallengeId != "wire-challenge" || input.TurnstileToken != "wire-token" || input.PasskeyAssertionJson != `{"id":"wire-credential"}` {
		s.t.Error("authorization lost proof fields")
	}
	deadline, ok := ctx.Deadline()
	if !ok || time.Until(deadline) < 10*time.Second {
		s.t.Error("verification RPC retained the generic two-second deadline")
	}
	return &pb.CheckinAuthorization{AuthorizationId: "wire-authorization", Day: "2026-11-15", ExpiresAt: 1794715320}, nil
}

func (s *checkinProofServer) Post(ctx context.Context, input *pb.CheckinLedgerPost) (*pb.CheckinLedgerReceipt, error) {
	s.identity(ctx)
	if input.OperationKey != s.key || input.AuthorizationId != "wire-authorization" || input.Day != "2026-11-15" || input.AmountUnits != 7 || input.TrustLevel != 4 {
		s.t.Error("posting lost original intention or authorization")
	}
	journal := int64(1)
	return &pb.CheckinLedgerReceipt{UserId: 11, AccountId: 22, Day: input.Day, AmountUnits: input.AmountUnits, JournalId: &journal, Timestamp: 1794715200}, nil
}

func TestCheckinVerificationRPCBinding(t *testing.T) {
	listener := bufconn.Listen(MaxMessage)
	server := grpc.NewServer()
	impl := &checkinProofServer{t: t, key: "checkin-wire-operation", user: strings.Repeat("u", 48), service: strings.Repeat("f", 48)}
	pb.RegisterCoreCheckinLedgerServer(server, impl)
	go server.Serve(listener)
	t.Cleanup(server.Stop)
	client := &Client{authorization: "Bearer " + impl.service, slots: make(chan struct{}, maxInFlight)}
	connection, e := grpc.NewClient("passthrough:///checkin-test", grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithContextDialer(func(context.Context, string) (net.Conn, error) { return listener.Dial() }),
		grpc.WithUnaryInterceptor(client.boundCall))
	if e != nil {
		t.Fatal(e)
	}
	client.connection = connection
	t.Cleanup(func() { client.Close() })
	ledger := CheckinLedger{Client: client}
	ctx := metadata.NewOutgoingContext(context.Background(), metadata.Pairs("authorization", "forged", "x-lmm-user-credential", "forged"))
	challenge, e := ledger.Challenge(ctx, impl.user, impl.key)
	if e != nil || challenge.OperationKey != impl.key || !challenge.PasskeyRequired || !json.Valid(challenge.PasskeyOptions) {
		t.Fatalf("challenge: %+v %v", challenge, e)
	}
	grant, e := ledger.Authorize(ctx, impl.user, checkins.ClaimRequest{OperationKey: impl.key, ChallengeID: challenge.ChallengeID, TurnstileToken: "wire-token", PasskeyAssertion: json.RawMessage(`{"id":"wire-credential"}`)})
	if e != nil || grant.ID != "wire-authorization" {
		t.Fatalf("authorization: %+v %v", grant, e)
	}
	receipt, e := ledger.Post(ctx, impl.user, checkins.Posting{Day: grant.Day, Amount: 7, Level: 4, AuthorizationID: grant.ID, OperationKey: impl.key})
	if e != nil || receipt.AmountUnits != 7 || receipt.JournalID == nil {
		t.Fatalf("receipt: %+v %v", receipt, e)
	}
}

func TestCheckinVerificationErrors(t *testing.T) {
	for _, tc := range []struct {
		code    codes.Code
		message string
		want    error
	}{
		{codes.FailedPrecondition, "CHECKIN_VERIFICATION_REQUIRED", checkins.ErrVerificationRequired},
		{codes.PermissionDenied, "CHECKIN_VERIFICATION_FAILED", checkins.ErrVerificationFailed},
		{codes.FailedPrecondition, "CHECKIN_PASSKEY_REQUIRED", checkins.ErrPasskeyRequired},
		{codes.ResourceExhausted, "rate limit", checkins.ErrRateLimited},
		{codes.Unavailable, "private upstream failure", checkins.ErrUnavailable},
		{codes.FailedPrecondition, "funding daily authorization expired", checkins.ErrConflict},
	} {
		if got := checkinError(status.Error(tc.code, tc.message)); got != tc.want {
			t.Fatalf("%s: %v", tc.message, got)
		}
	}
}
