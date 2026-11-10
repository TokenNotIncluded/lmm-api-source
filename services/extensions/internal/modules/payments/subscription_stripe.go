package payments

import (
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

//go:embed subscription_stripe_schema.sql
var SubscriptionStripeSchema string

const StripeSubscriptionAPIVersion = "2024-10-28.acacia"

type StripeSubscriptionCore interface {
	PrepareStripePurchase(context.Context, string, *pb.PrepareEPayPurchaseRequest) (*pb.StripeSubscriptionOrder, error)
	ConfirmStripePurchase(context.Context, string, string) (*pb.StripeSubscriptionOrder, error)
	ReadStripePurchase(context.Context, string, string) (*pb.StripeSubscriptionOrder, error)
	AcceptStripeSubscriptionEvent(context.Context, *pb.AcceptEPaySubscriptionProofRequest) (*pb.StripeSubscriptionEvent, error)
	GetStripeSubscriptionEvent(context.Context, *pb.GetEPaySubscriptionProofRequest) (*pb.StripeSubscriptionEvent, error)
}
type StripeHostedCheckout struct {
	ID        string `json:"id"`
	URL       string `json:"url"`
	ExpiresAt int64  `json:"expires_at"`
}
type StripePurchaseView struct {
	Order    *pb.StripeSubscriptionOrder
	Checkout *StripeHostedCheckout
}
type StripeSubscriptions struct {
	core         StripeSubscriptionCore
	db           *sql.DB
	adapters     map[int64]*Stripe
	newPurchases map[int64]bool
}

// Storage budgets are fresh per DB stage. Do not reuse one deadline across a
// potentially slow Core/provider call and then acknowledge a lost inbox write.
func (s *StripeSubscriptions) exec(ctx context.Context, q string, args ...any) (sql.Result, error) {
	child, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	return s.db.ExecContext(child, q, args...)
}
func (s *StripeSubscriptions) scan(ctx context.Context, q string, args []any, dest ...any) error {
	child, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	return s.db.QueryRowContext(child, q, args...).Scan(dest...)
}

// newPurchases is the independent SUB admission switch, NOT wallet availability.
// Disabled new purchases do not disable callbacks or recovery of issued orders.
func NewStripeSubscriptions(core StripeSubscriptionCore, db *sql.DB, adapters []*Stripe, newPurchases map[int64]bool) (*StripeSubscriptions, error) {
	if core == nil || db == nil {
		return nil, ErrInvalid
	}
	s := &StripeSubscriptions{core: core, db: db, adapters: map[int64]*Stripe{}, newPurchases: map[int64]bool{}}
	for _, a := range adapters {
		if a == nil || s.adapters[a.Channel().ID] != nil {
			return nil, ErrInvalid
		}
		copy := *a
		copy.cfg.APIVersion = StripeSubscriptionAPIVersion
		s.adapters[a.Channel().ID] = &copy
		s.newPurchases[a.Channel().ID] = newPurchases[a.Channel().ID]
	}
	// Read-only startup capability check: no hidden installation or write probe.
	ctx, cancel := context.WithTimeout(context.Background(), storageTimeout)
	defer cancel()
	var complete bool
	err := db.QueryRowContext(ctx, `SELECT count(*)=2 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='r' AND c.relname IN ('payment_stripe_subscription_checkouts','payment_stripe_subscription_inbox') AND has_schema_privilege(n.oid,'USAGE') AND has_table_privilege(c.oid,'SELECT') AND has_table_privilege(c.oid,'INSERT') AND has_table_privilege(c.oid,'UPDATE')`).Scan(&complete)
	if err != nil || !complete {
		return nil, ErrUnavailable
	}
	return s, nil
}
func (s *StripeSubscriptions) Prepare(ctx context.Context, user, key string, plan, channel int64) (*pb.StripeSubscriptionOrder, error) {
	if !validKey(key) || plan <= 0 || s.adapters[channel] == nil || !s.newPurchases[channel] {
		return nil, ErrInvalid
	}
	o, err := s.core.PrepareStripePurchase(ctx, user, &pb.PrepareEPayPurchaseRequest{Mutation: &pb.MutationContext{IdempotencyKey: key}, PlanId: plan, PaymentChannelId: channel})
	if err != nil {
		return nil, err
	}
	if !validStripeOrder(o) || o.Order.PlanId != plan || o.Order.PaymentChannelId != channel {
		return nil, ErrConflict
	}
	return o, nil
}
func validStripeOrder(o *pb.StripeSubscriptionOrder) bool {
	if o == nil || o.Order == nil || o.Price == nil {
		return false
	}
	b, p := o.Order, o.Price
	amount, err := strconv.ParseInt(p.AmountMinor, 10, 64)
	return err == nil && validAmount(amount) && strconv.FormatInt(amount, 10) == p.AmountMinor && subscriptionOrderID.MatchString(b.OrderId) && b.AccountId > 0 && len(b.PlanJson) > 0 && b.PlanId == p.PlanId && b.PlanRevision == p.PlanRevision && b.PaymentChannelId == p.ChannelId && b.Currency == "USD" && p.Currency == "USD" && b.AmountMinor == p.AmountMinor && strings.HasPrefix(p.ProviderPriceId, "price_")
}
func stripeFrozenHash(o *pb.StripeSubscriptionOrder) (string, error) {
	if !validStripeOrder(o) {
		return "", ErrConflict
	}
	price, err := (proto.MarshalOptions{Deterministic: true}).Marshal(o.Price)
	if err != nil {
		return "", ErrInvalid
	}
	return stableKey(o.Order.OrderId, strconv.FormatInt(o.Order.AccountId, 10), string(o.Order.PlanJson), string(price), o.Merchant, o.Environment)[4:], nil
}
func (s *StripeSubscriptions) Read(ctx context.Context, user, id string) (*StripePurchaseView, error) {
	if !subscriptionOrderID.MatchString(id) {
		return nil, ErrInvalid
	}
	o, err := s.core.ReadStripePurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validStripeOrder(o) || o.Order.OrderId != id {
		return nil, ErrConflict
	}
	view := &StripePurchaseView{Order: o}
	// Read stays available even with no configured provider or no checkout ACK.
	var c StripeHostedCheckout
	err = s.scan(ctx, `SELECT checkout_id,checkout_url,checkout_expires FROM payment_stripe_subscription_checkouts WHERE order_id=$1 AND checkout_id IS NOT NULL`, []any{id}, &c.ID, &c.URL, &c.ExpiresAt)
	if err == nil {
		view.Checkout = &c
	} else if !errors.Is(err, sql.ErrNoRows) {
		return nil, storageError(err)
	}
	return view, nil
}
func (s *StripeSubscriptions) Confirm(ctx context.Context, user, id string) (*StripePurchaseView, error) {
	if !subscriptionOrderID.MatchString(id) {
		return nil, ErrInvalid
	}
	prior, err := s.core.ReadStripePurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validStripeOrder(prior) || prior.Order.OrderId != id {
		return nil, ErrConflict
	}
	// Pausing purchases cannot strand an already-confirmed original checkout.
	// An unconfirmed quote is still a new purchase and requires SUB admission.
	if prior.Order.ConfirmedAtUnix == nil && !s.newPurchases[prior.Order.PaymentChannelId] {
		return nil, ErrDenied
	}
	o, err := s.core.ConfirmStripePurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validStripeOrder(o) || o.Order.OrderId != id || o.Order.ConfirmedAtUnix == nil {
		return nil, ErrConflict
	}
	a := s.adapters[o.Order.PaymentChannelId]
	if a == nil || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment {
		return nil, ErrUnavailable
	}
	frozen, err := stripeFrozenHash(o)
	if err != nil {
		return nil, err
	}
	// Persist dispatch identity before ANY upstream mutation. Immutable first
	// attempt defines the replay window even when Checkout's response is lost.
	encoded, err := proto.Marshal(o)
	if err != nil {
		return nil, ErrInvalid
	}
	if _, err = s.exec(ctx, `INSERT INTO payment_stripe_subscription_checkouts(order_id,channel_id,frozen_sha256,frozen_order) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING`, id, o.Price.ChannelId, frozen, encoded); err != nil {
		return nil, storageError(err)
	}
	var saved string
	var first time.Time
	var checkoutID sql.NullString
	if err = s.scan(ctx, `SELECT frozen_sha256,first_dispatch,checkout_id FROM payment_stripe_subscription_checkouts WHERE order_id=$1`, []any{id}, &saved, &first, &checkoutID); err != nil {
		return nil, storageError(err)
	}
	if saved != frozen {
		return nil, ErrConflict
	}
	if checkoutID.Valid {
		return s.Read(ctx, user, id)
	}
	checkout, err := a.findSubscriptionCheckout(ctx, o, "", first.Unix())
	if err != nil {
		return nil, err
	}
	if checkout == nil {
		// Never create a second session after Stripe may prune its idempotency key.
		// Keep original UNKNOWN; provider lookup / original callback can resolve it.
		if time.Since(first) >= a.ReplayWindow("") {
			return nil, ErrUnavailable
		}
		checkout, err = a.createSubscriptionCheckout(ctx, o)
		if err != nil {
			return nil, err
		}
	}
	if err = s.saveCheckout(ctx, id, *checkout); err != nil {
		return nil, err
	}
	return s.Read(ctx, user, id)
}
func (s *StripeSubscriptions) saveCheckout(ctx context.Context, id string, c StripeHostedCheckout) error {
	var saved string
	err := s.scan(ctx, `UPDATE payment_stripe_subscription_checkouts SET checkout_id=$2,checkout_url=$3,checkout_expires=$4,recovered_at=clock_timestamp() WHERE order_id=$1 AND (checkout_id IS NULL OR (checkout_id=$2 AND checkout_url=$3 AND checkout_expires=$4)) RETURNING checkout_id`, []any{id, c.ID, c.URL, c.ExpiresAt}, &saved)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrConflict
	}
	if err != nil {
		return storageError(err)
	}
	return nil
}
func stripeMetadata(o *pb.StripeSubscriptionOrder) map[string]string {
	return map[string]string{"lmm_subscription_order_id": o.Order.OrderId, "lmm_price_id": o.Price.ProviderPriceId, "lmm_merchant": o.Merchant}
}
func (a *Stripe) validateSubscriptionPrice(ctx context.Context, o *pb.StripeSubscriptionOrder) error {
	if !validStripeOrder(o) || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment {
		return ErrConflict
	}
	if err := a.merchant(ctx); err != nil {
		return err
	}
	raw, err := a.api(ctx, "GET", "prices/"+url.PathEscape(o.Price.ProviderPriceId), "", nil)
	if err != nil {
		return err
	}
	var p struct {
		ID, Object, Type, Currency string
		Active, Livemode           bool
		Amount                     int64 `json:"unit_amount"`
		Recurring                  struct {
			Interval string
			Count    int64 `json:"interval_count"`
		} `json:"recurring"`
	}
	if jsonObject(raw, &p) != nil || p.ID != o.Price.ProviderPriceId || p.Object != "price" || p.Type != "recurring" || !p.Active || p.Livemode != (o.Environment == "live") || !strings.EqualFold(p.Currency, o.Price.Currency) || strconv.FormatInt(p.Amount, 10) != o.Price.AmountMinor || p.Recurring.Interval != o.Price.Interval || p.Recurring.Count != o.Price.IntervalCount {
		return ErrConflict
	}
	return nil
}
func (a *Stripe) createSubscriptionCheckout(ctx context.Context, o *pb.StripeSubscriptionOrder) (*StripeHostedCheckout, error) {
	if err := a.validateSubscriptionPrice(ctx, o); err != nil {
		return nil, err
	}
	// URLs are frozen by root-only Core price configuration, not request input or
	// mutable wallet settings. Allow only the same safe production return shape.
	if _, err := secureURL(o.Price.SuccessUrl); err != nil {
		return nil, err
	}
	if _, err := secureURL(o.Price.CancelUrl); err != nil {
		return nil, err
	}
	f := url.Values{"mode": {"subscription"}, "line_items[0][price]": {o.Price.ProviderPriceId}, "line_items[0][quantity]": {"1"}, "client_reference_id": {o.Order.OrderId}, "success_url": {o.Price.SuccessUrl}, "cancel_url": {o.Price.CancelUrl}, "allow_promotion_codes": {"false"}, "automatic_tax[enabled]": {"false"}}
	f.Set("payment_method_types[0]", "card")
	for k, v := range stripeMetadata(o) {
		f.Set("metadata["+k+"]", v)
		f.Set("subscription_data[metadata]["+k+"]", v)
	}
	raw, err := a.api(ctx, "POST", "checkout/sessions", stableKey("stripe-subscription", o.Order.OrderId), f)
	if err != nil {
		return nil, err
	}
	return a.subscriptionCheckout(raw, o)
}
func (a *Stripe) subscriptionCheckout(raw []byte, o *pb.StripeSubscriptionOrder) (*StripeHostedCheckout, error) {
	var v struct {
		stripeObject
		Expires int64 `json:"expires_at"`
	}
	if jsonObject(raw, &v) != nil || v.Object != "checkout.session" || v.Mode != "subscription" || !strings.HasPrefix(v.ID, "cs_") || v.Reference != o.Order.OrderId || v.Livemode == nil || *v.Livemode != (o.Environment == "live") || !strings.EqualFold(v.Currency, o.Price.Currency) || strconv.FormatInt(v.Total, 10) != o.Price.AmountMinor || v.Expires <= 0 {
		return nil, ErrConflict
	}
	for k, value := range stripeMetadata(o) {
		if v.Metadata[k] != value {
			return nil, ErrConflict
		}
	}
	if v.URL == "" {
		return nil, ErrUnavailable
	}
	if !a.validCheckoutURL(v.URL) {
		return nil, ErrConflict
	}
	return &StripeHostedCheckout{ID: v.ID, URL: v.URL, ExpiresAt: v.Expires}, nil
}
func (a *Stripe) findSubscriptionCheckout(ctx context.Context, o *pb.StripeSubscriptionOrder, id string, first int64) (*StripeHostedCheckout, error) {
	if err := a.merchant(ctx); err != nil {
		return nil, err
	}
	if id != "" {
		raw, err := a.api(ctx, "GET", "checkout/sessions/"+url.PathEscape(id), "", nil)
		if err != nil {
			return nil, err
		}
		return a.subscriptionCheckout(raw, o)
	}
	cursor := ""
	for page := 0; page < 10; page++ {
		q := url.Values{"limit": {"100"}, "created[gte]": {strconv.FormatInt(first-5, 10)}}
		if cursor != "" {
			q.Set("starting_after", cursor)
		}
		raw, err := a.api(ctx, "GET", "checkout/sessions?"+q.Encode(), "", nil)
		if err != nil {
			return nil, err
		}
		var list struct {
			Data []json.RawMessage
			More bool `json:"has_more"`
		}
		if jsonObject(raw, &list) != nil {
			return nil, ErrInvalid
		}
		for _, item := range list.Data {
			var v stripeObject
			if jsonObject(item, &v) != nil || !validID(v.ID) {
				return nil, ErrInvalid
			}
			cursor = v.ID
			if v.Reference == o.Order.OrderId {
				return a.subscriptionCheckout(item, o)
			}
		}
		if !list.More {
			return nil, nil
		}
		if len(list.Data) == 0 {
			return nil, ErrUnavailable
		}
	}
	return nil, ErrUnavailable
}

type stripeCandidate struct {
	ID, Order, Event   string
	Channel            int64
	Payload, Signature []byte
}

func stripeCallbackIdentity(raw []byte) (string, string, error) {
	var envelope struct {
		ID, Type string
		Data     struct {
			Object struct {
				Metadata map[string]string
				Details  struct{ Metadata map[string]string } `json:"subscription_details"`
				Parent   struct {
					Details struct{ Metadata map[string]string } `json:"subscription_details"`
				}
			}
		}
	}
	if jsonObject(raw, &envelope) != nil || !validID(envelope.ID) || !strings.HasPrefix(envelope.ID, "evt_") {
		return "", "", ErrInvalid
	}
	m := envelope.Data.Object.Metadata
	if strings.HasPrefix(envelope.Type, "invoice.") {
		m = envelope.Data.Object.Details.Metadata
		if m == nil {
			m = envelope.Data.Object.Parent.Details.Metadata
		}
	}
	id := m["lmm_subscription_order_id"]
	if !subscriptionOrderID.MatchString(id) {
		return "", "", ErrInvalid
	}
	return id, envelope.ID, nil
}
func (s *StripeSubscriptions) Accept(ctx context.Context, channel int64, r *http.Request) (*pb.StripeSubscriptionEvent, error) {
	if s.adapters[channel] == nil {
		return nil, ErrNotFound
	}
	raw, sig, err := callbackBytes(r, "Stripe-Signature")
	if err != nil {
		return nil, err
	}
	order, event, err := stripeCallbackIdentity(raw)
	if err != nil {
		return nil, err
	}
	c := stripeCandidate{ID: stableKey("stripe-subscription-evidence", strconv.FormatInt(channel, 10), order, event, eventHash(raw), eventHash(sig)), Order: order, Event: event, Channel: channel, Payload: raw, Signature: sig}
	if _, err = s.exec(ctx, `INSERT INTO payment_stripe_subscription_inbox(evidence_id,order_id,channel_id,event_id,payload,signature) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING`, c.ID, c.Order, c.Channel, c.Event, c.Payload, c.Signature); err != nil {
		return nil, storageError(err)
	}
	return s.settle(ctx, c)
}
func (s *StripeSubscriptions) settle(ctx context.Context, c stripeCandidate) (*pb.StripeSubscriptionEvent, error) {
	digest := sha256.Sum256(c.Payload)
	receipt, err := s.core.GetStripeSubscriptionEvent(ctx, &pb.GetEPaySubscriptionProofRequest{OrderId: c.Order, ProviderEventId: c.Event, PayloadSha256: digest[:]})
	if status.Code(err) == codes.NotFound {
		receipt, err = s.core.AcceptStripeSubscriptionEvent(ctx, &pb.AcceptEPaySubscriptionProofRequest{OrderId: c.Order, SignedPayload: c.Payload, Signature: c.Signature})
		if status.Code(err) == codes.InvalidArgument {
			err = fmt.Errorf("%w: original Stripe proof", ErrEvidence)
		}
	}
	if err == nil && (receipt == nil || receipt.OrderId != c.Order || receipt.EventId != c.Event || hex.EncodeToString(receipt.PayloadSha256) != hex.EncodeToString(digest[:])) {
		err = ErrConflict
	}
	if err == nil {
		encoded, marshalErr := proto.Marshal(receipt)
		if marshalErr != nil {
			return nil, ErrUnavailable
		}
		_, err = s.exec(ctx, `UPDATE payment_stripe_subscription_inbox SET state='accepted',result=$2,error_code='' WHERE evidence_id=$1 AND payload=$3`, c.ID, encoded, c.Payload)
		if err != nil {
			return nil, storageError(err)
		}
		return receipt, nil
	}
	state := "pending"
	if errors.Is(err, ErrEvidence) {
		state = "rejected"
	}
	_, saveErr := s.exec(ctx, `UPDATE payment_stripe_subscription_inbox SET state=$2,attempts=least(attempts+1,1000000),next_attempt=clock_timestamp()+(least(attempts,59)+1)*interval '1 second',error_code=$3 WHERE evidence_id=$1 AND state<>'accepted'`, c.ID, state, errorCode(err))
	if saveErr != nil {
		return nil, storageError(saveErr)
	}
	return nil, err
}
func (s *StripeSubscriptions) Recover(ctx context.Context, limit int) error {
	if limit < 1 || limit > 100 {
		return ErrInvalid
	}
	dbCtx, dbCancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(dbCtx, `SELECT evidence_id,order_id,channel_id,event_id,payload,signature FROM payment_stripe_subscription_inbox WHERE state='pending' AND next_attempt<=clock_timestamp() ORDER BY next_attempt,evidence_id LIMIT $1`, limit)
	if err != nil {
		dbCancel()
		return storageError(err)
	}
	var candidates []stripeCandidate
	for rows.Next() {
		var c stripeCandidate
		if err = rows.Scan(&c.ID, &c.Order, &c.Channel, &c.Event, &c.Payload, &c.Signature); err != nil {
			rows.Close()
			dbCancel()
			return storageError(err)
		}
		candidates = append(candidates, c)
	}
	err = rows.Err()
	rows.Close()
	dbCancel()
	if err != nil {
		return storageError(err)
	}
	var result error
	for _, c := range candidates {
		if _, err = s.settle(ctx, c); err != nil {
			result = errors.Join(result, err)
		}
	}
	return errors.Join(result, s.recoverCheckouts(ctx, limit))
}

func (s *StripeSubscriptions) recoverCheckouts(ctx context.Context, limit int) error {
	dbCtx, cancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(dbCtx, `SELECT order_id,frozen_sha256,frozen_order,first_dispatch FROM payment_stripe_subscription_checkouts WHERE checkout_id IS NULL AND next_lookup<=clock_timestamp() ORDER BY next_lookup,order_id LIMIT $1`, limit)
	if err != nil {
		cancel()
		return storageError(err)
	}
	type pending struct {
		id, hash string
		raw      []byte
		first    time.Time
	}
	var all []pending
	for rows.Next() {
		var p pending
		if err = rows.Scan(&p.id, &p.hash, &p.raw, &p.first); err != nil {
			rows.Close()
			cancel()
			return storageError(err)
		}
		all = append(all, p)
	}
	err = rows.Err()
	rows.Close()
	cancel()
	if err != nil {
		return storageError(err)
	}
	var result error
	for _, p := range all {
		var o pb.StripeSubscriptionOrder
		err = proto.Unmarshal(p.raw, &o)
		if err == nil {
			var hash string
			hash, err = stripeFrozenHash(&o)
			if hash != p.hash || o.Order.OrderId != p.id {
				err = ErrConflict
			}
		}
		if err == nil {
			a := s.adapters[o.Price.ChannelId]
			if a == nil || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment {
				err = ErrUnavailable
			} else {
				var c *StripeHostedCheckout
				c, err = a.findSubscriptionCheckout(ctx, &o, "", p.first.Unix())
				if err == nil && c != nil {
					err = s.saveCheckout(ctx, p.id, *c)
				} else if err == nil {
					err = ErrUnavailable
				}
			}
		}
		// Recovery never creates another checkout, irrespective of elapsed time.
		// Defer UNKNOWN fairly; absence from a bounded provider page is not unpaid.
		if err != nil {
			_, saveErr := s.exec(ctx, `UPDATE payment_stripe_subscription_checkouts SET lookup_attempts=least(lookup_attempts+1,1000000),next_lookup=clock_timestamp()+(least(lookup_attempts,59)+1)*interval '1 second' WHERE order_id=$1 AND checkout_id IS NULL`, p.id)
			result = errors.Join(result, err, storageError(saveErr))
		}
	}
	return result
}
func (s *StripeSubscriptions) CancelRenewal(ctx context.Context, user, id string) error {
	view, err := s.Read(ctx, user, id)
	if err != nil {
		return err
	}
	o := view.Order
	if o.ProviderSubscriptionId == "" {
		return ErrConflict
	}
	a := s.adapters[o.Price.ChannelId]
	if a == nil || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment {
		return ErrUnavailable
	}
	if err = a.merchant(ctx); err != nil {
		return err
	}
	// The original subscription ID comes only from Core's verified signed event.
	// An API reply is an observation: only the signed lifecycle event updates Core.
	_, err = a.api(ctx, "POST", "subscriptions/"+url.PathEscape(o.ProviderSubscriptionId), stableKey("stripe-cancel-renewal", id), url.Values{"cancel_at_period_end": {"true"}})
	return err
}
