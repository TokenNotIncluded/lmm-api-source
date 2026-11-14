package payments

import (
	"context"
	"crypto/sha256"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type epaySubscriptionRecoveryCore struct {
	SubscriptionCore
	order     *pb.EPaySubscriptionOrder
	confirms  int
	prepares  int
	acceptErr error
	accepted  int
	grants    int
	proof     *pb.EPaySubscriptionProof
}

func (c *epaySubscriptionRecoveryCore) PrepareEPayPurchase(_ context.Context, _ string, r *pb.PrepareEPayPurchaseRequest) (*pb.EPaySubscriptionOrder, error) {
	c.prepares++
	if r.PlanId != c.order.PlanId || r.PaymentChannelId != c.order.PaymentChannelId || r.Mutation == nil || r.Mutation.IdempotencyKey != "original_key_0001" {
		return nil, ErrConflict
	}
	return c.order, nil
}
func (c *epaySubscriptionRecoveryCore) ReadEPayPurchase(context.Context, string, string) (*pb.EPaySubscriptionOrder, error) {
	return c.order, nil
}
func (c *epaySubscriptionRecoveryCore) ConfirmEPayPurchase(context.Context, string, string) (*pb.EPayCheckout, error) {
	c.confirms++
	return &pb.EPayCheckout{OrderId: c.order.OrderId, Method: "GET", CheckoutUrl: "https://pay.example.com/submit.php?frozen=checkout"}, nil
}
func (c *epaySubscriptionRecoveryCore) GetEPaySubscriptionProof(context.Context, *pb.GetEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error) {
	if c.proof == nil {
		return nil, status.Error(codes.NotFound, "not accepted")
	}
	return c.proof, nil
}
func (c *epaySubscriptionRecoveryCore) AcceptEPaySubscriptionProof(_ context.Context, r *pb.AcceptEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error) {
	c.accepted++
	if c.acceptErr != nil {
		return nil, c.acceptErr
	}
	values, _ := url.ParseQuery(string(r.SignedPayload))
	hash := sha256.Sum256(r.SignedPayload)
	c.proof = &pb.EPaySubscriptionProof{OrderId: r.OrderId, ProofId: "proof_original", ProviderEventId: stableKey(values.Get("trade_no"), "TRADE_SUCCESS"), PayloadSha256: hash[:], AcceptedAtUnix: 1}
	return c.proof, nil
}
func (c *epaySubscriptionRecoveryCore) ApplyEPaySubscriptionGrant(_ context.Context, r *pb.ApplyEPaySubscriptionGrantRequest) (*pb.EPaySubscriptionGrant, error) {
	c.grants++
	return &pb.EPaySubscriptionGrant{OrderId: r.OrderId, ProofId: r.ProofId, OperationKey: r.OperationKey, SubscriptionId: 1, GrantedAtUnix: 1, EndsAtUnix: 2}, nil
}

type epaySubscriptionRecoveryInbox struct {
	entry    SubscriptionEvidence
	complete bool
	retries  int
}

func (i *epaySubscriptionRecoveryInbox) PutSubscriptionEvidence(_ context.Context, e Event) (SubscriptionEvidence, error) {
	i.entry = SubscriptionEvidence{ID: "candidate", OrderID: e.OrderID, EventID: e.Evidence.EventID, Payload: e.Evidence.Payload, Signature: e.Evidence.Signature}
	return i.entry, nil
}
func (i *epaySubscriptionRecoveryInbox) PendingSubscriptionEvidence(context.Context, int) ([]SubscriptionEvidence, error) {
	if i.complete {
		return nil, nil
	}
	return []SubscriptionEvidence{i.entry}, nil
}
func (i *epaySubscriptionRecoveryInbox) CompleteSubscriptionEvidence(context.Context, SubscriptionEvidence, string, int64) error {
	i.complete = true
	return nil
}
func (i *epaySubscriptionRecoveryInbox) RetrySubscriptionEvidence(context.Context, SubscriptionEvidence, error) error {
	i.retries++
	return nil
}

func epaySubscriptionRecoveryFixture(t *testing.T) (*EPaySubscriptions, *epaySubscriptionRecoveryCore, *epaySubscriptionRecoveryInbox) {
	t.Helper()
	a, err := NewCoreEPay(EPayConfig{Channel: Channel{ID: 2, Provider: "epay", Merchant: "1001", Environment: "test", Currency: "CNY", Enabled: false}, PaymentType: "wxpay"}, epaySignerFunc(func(context.Context, int64) (Checkout, error) {
		t.Fatal("wallet signing invoked")
		return Checkout{}, ErrInvalid
	}))
	if err != nil {
		t.Fatal(err)
	}
	c := &epaySubscriptionRecoveryCore{order: &pb.EPaySubscriptionOrder{OrderId: "sub_" + strings.Repeat("a", 64), AccountId: 1, PlanId: 1, PlanRevision: 1, PaymentChannelId: 2, Currency: "CNY", AmountMinor: "1000", ExpiresAtUnix: 1, PlanJson: []byte(`{"id":1}`)}}
	inbox := &epaySubscriptionRecoveryInbox{}
	s, err := NewEPaySubscriptions(c, inbox, []*EPay{a}, nil)
	if err != nil {
		t.Fatal(err)
	}
	return s, c, inbox
}

func TestEPaySubscriptionDisabledAdmissionBlocksFirstCheckoutPreservesConfirmed(t *testing.T) {
	s, c, _ := epaySubscriptionRecoveryFixture(t)
	if _, err := s.Prepare(context.Background(), "user", "original_key_0001", 1, 2); !errors.Is(err, ErrInvalid) {
		t.Fatalf("disabled admission: %v", err)
	}
	if _, err := s.Confirm(context.Background(), "user", c.order.OrderId); !errors.Is(err, ErrUnavailable) || c.confirms != 0 {
		t.Fatalf("disabled first checkout: calls=%d err=%v", c.confirms, err)
	}
	confirmed := int64(1)
	c.order.ConfirmedAtUnix = &confirmed
	checkout, err := s.Confirm(context.Background(), "user", c.order.OrderId)
	if err != nil || checkout.OrderId != c.order.OrderId || c.confirms != 1 {
		t.Fatalf("confirmed checkout lost: %v %v", checkout, err)
	}
}

func TestEPaySubscriptionChangedTypeCallbacksReachFrozenCoreAndRecover(t *testing.T) {
	for _, method := range []string{http.MethodGet, http.MethodPost} {
		t.Run(method, func(t *testing.T) {
			s, c, inbox := epaySubscriptionRecoveryFixture(t)
			values := epayValues()
			values.Set("out_trade_no", c.order.OrderId)
			values.Set("sign", epaySign(values, "fixture-merchant-key"))
			// A historical alipay checkout, with a currently disabled wxpay adapter.
			raw := values.Encode()
			c.acceptErr = status.Error(codes.Unavailable, "synthetic Core outage")
			req := httptest.NewRequest(method, "/callbacks/2", strings.NewReader(raw))
			if method == http.MethodGet {
				req = httptest.NewRequest(method, "/callbacks/2?"+raw, nil)
			} else {
				req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
			}
			w := httptest.NewRecorder()
			s.Handler().ServeHTTP(w, req)
			if w.Code == 200 || c.accepted != 1 || inbox.retries != 1 || string(inbox.entry.Payload) != raw || c.grants != 0 {
				t.Fatalf("old callback failed before Core or outage ACKed: status=%d accepts=%d retries=%d", w.Code, c.accepted, inbox.retries)
			}
			// This fake tests forwarding/recovery only, never actual signature authority.
			c.acceptErr = nil
			recovered, err := NewEPaySubscriptions(c, inbox, []*EPay{s.adapters[2]}, nil)
			if err != nil {
				t.Fatal(err)
			}
			if err = recovered.Recover(context.Background(), 1); err != nil || !inbox.complete || c.grants != 1 {
				t.Fatalf("original recovery lost: %v complete=%v grants=%d", err, inbox.complete, c.grants)
			}
			// Wallet callbacks must retain their current payment-type binding.
			walletReq := httptest.NewRequest(http.MethodGet, "/callbacks/2?"+raw, nil)
			if _, err = s.adapters[2].Verify(walletReq, time.Now()); !errors.Is(err, ErrDenied) {
				t.Fatalf("wallet type validation changed: %v", err)
			}
		})
	}
}

func TestEPaySubscriptionChangedTypeStillRequiresCoreAcceptance(t *testing.T) {
	s, c, inbox := epaySubscriptionRecoveryFixture(t)
	c.acceptErr = status.Error(codes.InvalidArgument, "original type/signature rejected")
	values := epayValues()
	values.Set("out_trade_no", c.order.OrderId)
	req := httptest.NewRequest(http.MethodGet, "/callbacks/2?"+values.Encode(), nil)
	w := httptest.NewRecorder()
	s.Handler().ServeHTTP(w, req)
	if w.Code == http.StatusOK || c.accepted != 1 || c.grants != 0 || inbox.complete || inbox.retries != 1 {
		t.Fatalf("Core rejection bypassed: status=%d accepts=%d grants=%d", w.Code, c.accepted, c.grants)
	}
}

func TestEPaySubscriptionAdmissionDefaultsClosedAndIndependentOfWallet(t *testing.T) {
	for _, wallet := range []bool{false, true} {
		for _, admission := range []map[int64]bool{nil, {}, {2: false}, {2: true}} {
			s, c, inbox := epaySubscriptionRecoveryFixture(t)
			a := *s.adapters[2]
			a.cfg.Channel.Enabled = wallet
			s, err := NewEPaySubscriptions(c, inbox, []*EPay{&a}, admission)
			if err != nil {
				t.Fatal(err)
			}
			// Constructor freezes configuration rather than retaining the caller map.
			enabled := admission[2]
			if admission != nil {
				admission[2] = !enabled
			}

			_, err = s.Prepare(context.Background(), "user", "original_key_0001", 1, 2)
			if enabled {
				if err != nil || c.prepares != 1 {
					t.Fatalf("SUB prepare enabled wallet=%v: calls=%d err=%v", wallet, c.prepares, err)
				}
			} else if !errors.Is(err, ErrInvalid) || c.prepares != 0 {
				t.Fatalf("SUB prepare disabled wallet=%v: calls=%d err=%v", wallet, c.prepares, err)
			}
			_, err = s.Confirm(context.Background(), "user", c.order.OrderId)
			if enabled {
				if err != nil || c.confirms != 1 {
					t.Fatalf("SUB enabled wallet=%v: calls=%d err=%v", wallet, c.confirms, err)
				}
			} else if !errors.Is(err, ErrUnavailable) || c.confirms != 0 {
				t.Fatalf("SUB disabled wallet=%v: calls=%d err=%v", wallet, c.confirms, err)
			}
		}
	}
}
