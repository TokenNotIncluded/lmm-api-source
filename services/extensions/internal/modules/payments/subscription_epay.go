package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	_ "embed"
	"errors"
	"net/http"
	"regexp"
	"strconv"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

//go:embed subscription_epay_schema.sql
var SubscriptionEPaySchema string
var subscriptionOrderID = regexp.MustCompile(`^sub_[a-f0-9]{64}$`)

// SubscriptionCore deliberately has no wallet credit operation.
type SubscriptionCore interface {
	PrepareEPayPurchase(context.Context, string, *pb.PrepareEPayPurchaseRequest) (*pb.EPaySubscriptionOrder, error)
	ConfirmEPayPurchase(context.Context, string, string) (*pb.EPayCheckout, error)
	ReadEPayPurchase(context.Context, string, string) (*pb.EPaySubscriptionOrder, error)
	CancelEPayPurchase(context.Context, string, string) (*pb.EPaySubscriptionOrder, error)
	AcceptEPaySubscriptionProof(context.Context, *pb.AcceptEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error)
	GetEPaySubscriptionProof(context.Context, *pb.GetEPaySubscriptionProofRequest) (*pb.EPaySubscriptionProof, error)
	ApplyEPaySubscriptionGrant(context.Context, *pb.ApplyEPaySubscriptionGrantRequest) (*pb.EPaySubscriptionGrant, error)
}
type SubscriptionEvidence struct {
	ID, OrderID, EventID, Digest string
	Payload, Signature           []byte
}
type SubscriptionInbox interface {
	PutSubscriptionEvidence(context.Context, Event) (SubscriptionEvidence, error)
	PendingSubscriptionEvidence(context.Context, int) ([]SubscriptionEvidence, error)
	CompleteSubscriptionEvidence(context.Context, SubscriptionEvidence, string, int64) error
	RetrySubscriptionEvidence(context.Context, SubscriptionEvidence, error) error
}
type EPaySubscriptions struct {
	core     SubscriptionCore
	inbox    SubscriptionInbox
	adapters map[int64]*EPay
}

func NewEPaySubscriptions(core SubscriptionCore, inbox SubscriptionInbox, adapters []*EPay) (*EPaySubscriptions, error) {
	if core == nil || inbox == nil || len(adapters) == 0 {
		return nil, ErrInvalid
	}
	s := &EPaySubscriptions{core: core, inbox: inbox, adapters: map[int64]*EPay{}}
	for _, a := range adapters {
		// Only Core holds the MD5 key that can sign both checkout and callbacks.
		if a == nil || a.signer == nil || s.adapters[a.Channel().ID] != nil {
			return nil, ErrInvalid
		}
		s.adapters[a.Channel().ID] = a
	}
	return s, nil
}
func (s *EPaySubscriptions) Prepare(ctx context.Context, user, key string, plan, channel int64) (*pb.EPaySubscriptionOrder, error) {
	a := s.adapters[channel]
	if a == nil || !a.Channel().Enabled || !validKey(key) || plan <= 0 {
		return nil, ErrInvalid
	}
	o, err := s.core.PrepareEPayPurchase(ctx, user, &pb.PrepareEPayPurchaseRequest{Mutation: &pb.MutationContext{IdempotencyKey: key}, PlanId: plan, PaymentChannelId: channel})
	if err != nil {
		return nil, err
	}
	if !validSubscriptionOrder(o) || o.PlanId != plan || o.PaymentChannelId != channel || o.Currency != a.Channel().Currency {
		return nil, ErrConflict
	}
	return o, nil
}
func validSubscriptionOrder(o *pb.EPaySubscriptionOrder) bool {
	if o == nil || !subscriptionOrderID.MatchString(o.OrderId) || o.AccountId <= 0 || o.PlanId <= 0 || o.PlanRevision <= 0 || o.PaymentChannelId <= 0 || len(o.PlanJson) == 0 || o.ExpiresAtUnix <= 0 {
		return false
	}
	n, err := strconv.ParseInt(o.AmountMinor, 10, 64)
	return err == nil && strconv.FormatInt(n, 10) == o.AmountMinor && validAmount(n) && (o.Currency == "CNY" || o.Currency == "LDC")
}
func (s *EPaySubscriptions) Read(ctx context.Context, user, id string) (*pb.EPaySubscriptionOrder, error) {
	if !subscriptionOrderID.MatchString(id) {
		return nil, ErrInvalid
	}
	o, err := s.core.ReadEPayPurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validSubscriptionOrder(o) || o.OrderId != id {
		return nil, ErrConflict
	}
	return o, nil
}
func (s *EPaySubscriptions) Confirm(ctx context.Context, user, id string) (*pb.EPayCheckout, error) {
	o, err := s.Read(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if s.adapters[o.PaymentChannelId] == nil {
		return nil, ErrUnavailable
	}
	c, err := s.core.ConfirmEPayPurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	method := "GET"
	if o.Currency == "LDC" {
		method = "POST"
	}
	if c == nil || c.OrderId != id || c.IntentId != 0 || c.Method != method {
		return nil, ErrConflict
	}
	if _, err = secureURL(c.CheckoutUrl); err != nil {
		return nil, ErrConflict
	}
	return c, nil
}
func (s *EPaySubscriptions) Cancel(ctx context.Context, user, id string) (*pb.EPaySubscriptionOrder, error) {
	if !subscriptionOrderID.MatchString(id) {
		return nil, ErrInvalid
	}
	o, err := s.core.CancelEPayPurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validSubscriptionOrder(o) || o.OrderId != id || o.CanceledAtUnix == nil {
		return nil, ErrConflict
	}
	return o, nil
}

// Input is durably queued before Core calls; ACK follows the durable grant receipt.
func (s *EPaySubscriptions) Accept(ctx context.Context, channel int64, r *http.Request) error {
	a := s.adapters[channel]
	if a == nil {
		return ErrNotFound
	}
	event, err := a.Verify(r, time.Now())
	if err != nil {
		return err
	}
	if event.Type != Paid || !subscriptionOrderID.MatchString(event.OrderID) {
		return ErrInvalid
	}
	entry, err := s.inbox.PutSubscriptionEvidence(ctx, event)
	if err != nil {
		return err
	}
	err = s.settle(ctx, entry)
	if err != nil {
		return errors.Join(err, s.inbox.RetrySubscriptionEvidence(ctx, entry, err))
	}
	return nil
}
func (s *EPaySubscriptions) settle(ctx context.Context, e SubscriptionEvidence) error {
	hash := sha256.Sum256(e.Payload)
	p, err := s.core.GetEPaySubscriptionProof(ctx, &pb.GetEPaySubscriptionProofRequest{OrderId: e.OrderID, ProviderEventId: e.EventID, PayloadSha256: hash[:]})
	if status.Code(err) == codes.NotFound || errors.Is(err, ErrNotFound) {
		p, err = s.core.AcceptEPaySubscriptionProof(ctx, &pb.AcceptEPaySubscriptionProofRequest{OrderId: e.OrderID, SignedPayload: e.Payload, Signature: e.Signature})
		if status.Code(err) == codes.InvalidArgument {
			return errors.Join(ErrEvidence, err)
		}
	}
	if err != nil {
		return err
	}
	if p == nil || p.ProofId == "" || p.OrderId != e.OrderID || p.ProviderEventId != e.EventID || p.AcceptedAtUnix <= 0 || !bytes.Equal(p.PayloadSha256, hash[:]) {
		return ErrConflict
	}
	key := stableKey("subscription-grant", e.OrderID)
	g, err := s.core.ApplyEPaySubscriptionGrant(ctx, &pb.ApplyEPaySubscriptionGrantRequest{OrderId: e.OrderID, ProofId: p.ProofId, OperationKey: key})
	if err != nil {
		return err
	}
	if g == nil || g.OrderId != e.OrderID || g.ProofId != p.ProofId || g.OperationKey != key || g.SubscriptionId <= 0 || g.GrantedAtUnix <= 0 || g.EndsAtUnix <= g.GrantedAtUnix {
		return ErrConflict
	}
	return s.inbox.CompleteSubscriptionEvidence(ctx, e, p.ProofId, g.SubscriptionId)
}
func (s *EPaySubscriptions) Recover(ctx context.Context, limit int) error {
	if limit < 1 || limit > 100 {
		return ErrInvalid
	}
	entries, err := s.inbox.PendingSubscriptionEvidence(ctx, limit)
	if err != nil {
		return err
	}
	var failures []error
	for _, e := range entries {
		if err = s.settle(ctx, e); err != nil {
			failures = append(failures, errors.Join(err, s.inbox.RetrySubscriptionEvidence(ctx, e, err)))
		}
	}
	return errors.Join(failures...)
}
