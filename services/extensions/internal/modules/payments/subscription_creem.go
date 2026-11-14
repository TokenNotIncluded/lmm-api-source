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

//go:embed subscription_creem_schema.sql
var SubscriptionCreemSchema string

type CreemSubscriptionCore interface {
	ResolveCreemSubscriptionOrder(context.Context, *pb.CreemSubscriptionLookup) (*pb.EPaySubscriptionOrderRequest, error)
	PrepareCreemPurchase(context.Context, string, *pb.PrepareEPayPurchaseRequest) (*pb.CreemSubscriptionOrder, error)
	ConfirmCreemPurchase(context.Context, string, string) (*pb.CreemSubscriptionOrder, error)
	ReadCreemPurchase(context.Context, string, string) (*pb.CreemSubscriptionOrder, error)
	AcceptCreemSubscriptionEvent(context.Context, *pb.AcceptCreemSubscriptionEventRequest) (*pb.CreemSubscriptionEvent, error)
	GetCreemSubscriptionEvent(context.Context, *pb.GetCreemSubscriptionEventRequest) (*pb.CreemSubscriptionEvent, error)
}
type CreemHostedCheckout struct {
	ID        string `json:"id"`
	URL       string `json:"url"`
	ExpiresAt int64  `json:"expires_at"`
}
type CreemPurchaseView struct {
	Order    *pb.CreemSubscriptionOrder
	Checkout *CreemHostedCheckout
}
type CreemSubscriptions struct {
	core         CreemSubscriptionCore
	db           *sql.DB
	adapters     map[int64]*Creem
	newPurchases map[int64]bool
}

// Storage budgets are fresh per DB stage. Do not reuse one deadline across a
// potentially slow Core/provider call and then acknowledge a lost inbox write.
func (s *CreemSubscriptions) exec(ctx context.Context, q string, args ...any) (sql.Result, error) {
	child, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	return s.db.ExecContext(child, q, args...)
}
func (s *CreemSubscriptions) scan(ctx context.Context, q string, args []any, dest ...any) error {
	child, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	return s.db.QueryRowContext(child, q, args...).Scan(dest...)
}

// newPurchases is the independent SUB admission switch, NOT wallet availability.
// Disabled new purchases do not disable callbacks or recovery of issued orders.
func NewCreemSubscriptions(core CreemSubscriptionCore, db *sql.DB, adapters []*Creem, newPurchases map[int64]bool) (*CreemSubscriptions, error) {
	if core == nil || db == nil {
		return nil, ErrInvalid
	}
	s := &CreemSubscriptions{core: core, db: db, adapters: map[int64]*Creem{}, newPurchases: map[int64]bool{}}
	for _, a := range adapters {
		if a == nil || s.adapters[a.Channel().ID] != nil {
			return nil, ErrInvalid
		}
		copy := *a
		s.adapters[a.Channel().ID] = &copy
		s.newPurchases[a.Channel().ID] = newPurchases[a.Channel().ID]
	}
	// Read-only startup capability check: no hidden installation or write probe.
	ctx, cancel := context.WithTimeout(context.Background(), storageTimeout)
	defer cancel()
	var complete bool
	err := db.QueryRowContext(ctx, `SELECT count(*)=2 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='r' AND c.relname IN ('payment_creem_subscription_checkouts','payment_creem_subscription_inbox') AND has_schema_privilege(n.oid,'USAGE') AND has_table_privilege(c.oid,'SELECT') AND has_table_privilege(c.oid,'INSERT') AND has_table_privilege(c.oid,'UPDATE')`).Scan(&complete)
	if err != nil || !complete {
		return nil, ErrUnavailable
	}
	return s, nil
}
func (s *CreemSubscriptions) Prepare(ctx context.Context, user, key string, plan, channel int64) (*pb.CreemSubscriptionOrder, error) {
	if !validKey(key) || plan <= 0 || s.adapters[channel] == nil || !s.newPurchases[channel] {
		return nil, ErrInvalid
	}
	o, err := s.core.PrepareCreemPurchase(ctx, user, &pb.PrepareEPayPurchaseRequest{Mutation: &pb.MutationContext{IdempotencyKey: key}, PlanId: plan, PaymentChannelId: channel})
	if err != nil {
		return nil, err
	}
	if !validCreemOrder(o) || o.Order.PlanId != plan || o.Order.PaymentChannelId != channel {
		return nil, ErrConflict
	}
	return o, nil
}
func validCreemOrder(o *pb.CreemSubscriptionOrder) bool {
	if o == nil || o.Order == nil || o.Price == nil {
		return false
	}
	b, p := o.Order, o.Price
	amount, err := strconv.ParseInt(p.AmountMinor, 10, 64)
	return err == nil && validAmount(amount) && strconv.FormatInt(amount, 10) == p.AmountMinor && subscriptionOrderID.MatchString(b.OrderId) && b.AccountId > 0 && len(b.PlanJson) > 0 && b.PlanId == p.PlanId && b.PlanRevision == p.PlanRevision && b.PaymentChannelId == p.ChannelId && b.Currency == "USD" && p.Currency == "USD" && b.AmountMinor == p.AmountMinor && strings.HasPrefix(p.ProviderPriceId, "prod_")
}
func creemFrozenHash(o *pb.CreemSubscriptionOrder) (string, error) {
	if !validCreemOrder(o) {
		return "", ErrConflict
	}
	price, err := (proto.MarshalOptions{Deterministic: true}).Marshal(o.Price)
	if err != nil {
		return "", ErrInvalid
	}
	return stableKey(o.Order.OrderId, strconv.FormatInt(o.Order.AccountId, 10), string(o.Order.PlanJson), string(price), o.Merchant, o.Environment)[4:], nil
}
func (s *CreemSubscriptions) Read(ctx context.Context, user, id string) (*CreemPurchaseView, error) {
	if !subscriptionOrderID.MatchString(id) {
		return nil, ErrInvalid
	}
	o, err := s.core.ReadCreemPurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validCreemOrder(o) || o.Order.OrderId != id {
		return nil, ErrConflict
	}
	view := &CreemPurchaseView{Order: o}
	// Read stays available even with no configured provider or no checkout ACK.
	var c CreemHostedCheckout
	err = s.scan(ctx, `SELECT checkout_id,checkout_url,checkout_expires FROM payment_creem_subscription_checkouts WHERE order_id=$1 AND checkout_id IS NOT NULL`, []any{id}, &c.ID, &c.URL, &c.ExpiresAt)
	if err == nil {
		view.Checkout = &c
	} else if !errors.Is(err, sql.ErrNoRows) {
		return nil, storageError(err)
	}
	return view, nil
}
func (s *CreemSubscriptions) Confirm(ctx context.Context, user, id string) (*CreemPurchaseView, error) {
	if !subscriptionOrderID.MatchString(id) {
		return nil, ErrInvalid
	}
	prior, err := s.core.ReadCreemPurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validCreemOrder(prior) || prior.Order.OrderId != id {
		return nil, ErrConflict
	}
	// Pausing purchases cannot strand an already-confirmed original checkout.
	// An unconfirmed quote is still a new purchase and requires SUB admission.
	if prior.Order.ConfirmedAtUnix == nil && !s.newPurchases[prior.Order.PaymentChannelId] {
		return nil, ErrDenied
	}
	o, err := s.core.ConfirmCreemPurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validCreemOrder(o) || o.Order.OrderId != id || o.Order.ConfirmedAtUnix == nil {
		return nil, ErrConflict
	}
	a := s.adapters[o.Order.PaymentChannelId]
	if a == nil || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment {
		return nil, ErrUnavailable
	}
	frozen, err := creemFrozenHash(o)
	if err != nil {
		return nil, err
	}
	// Persist dispatch identity before ANY upstream mutation. Immutable first
	// attempt defines the replay window even when Checkout's response is lost.
	encoded, err := proto.Marshal(o)
	if err != nil {
		return nil, ErrInvalid
	}
	inserted, err := s.exec(ctx, `INSERT INTO payment_creem_subscription_checkouts(order_id,channel_id,frozen_sha256,frozen_order) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING`, id, o.Price.ChannelId, frozen, encoded)
	if err != nil {
		return nil, storageError(err)
	}
	affected, err := inserted.RowsAffected()
	if err != nil {
		return nil, storageError(err)
	}
	var saved string
	var first time.Time
	var checkoutID sql.NullString
	if err = s.scan(ctx, `SELECT frozen_sha256,first_dispatch,checkout_id FROM payment_creem_subscription_checkouts WHERE order_id=$1`, []any{id}, &saved, &first, &checkoutID); err != nil {
		return nil, storageError(err)
	}
	if saved != frozen {
		return nil, ErrConflict
	}
	if checkoutID.Valid {
		return s.Read(ctx, user, id)
	}
	// Creem documents request_id deduplication but no retention window or
	// lookup-by-request endpoint. An ambiguous dispatch must never be reposted.
	if affected == 0 {
		return nil, ErrUnavailable
	}
	checkout, err := a.createSubscriptionCheckout(ctx, o)
	if err != nil {
		return nil, err
	}
	if err = s.saveCheckout(ctx, id, *checkout); err != nil {
		return nil, err
	}
	return s.Read(ctx, user, id)
}
func (s *CreemSubscriptions) saveCheckout(ctx context.Context, id string, c CreemHostedCheckout) error {
	var saved string
	err := s.scan(ctx, `UPDATE payment_creem_subscription_checkouts SET checkout_id=$2,checkout_url=$3,checkout_expires=$4,recovered_at=clock_timestamp() WHERE order_id=$1 AND (checkout_id IS NULL OR (checkout_id=$2 AND checkout_url=$3 AND checkout_expires=$4)) RETURNING checkout_id`, []any{id, c.ID, c.URL, c.ExpiresAt}, &saved)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrConflict
	}
	if err != nil {
		return storageError(err)
	}
	return nil
}
func creemMetadata(o *pb.CreemSubscriptionOrder) map[string]string {
	return map[string]string{"lmm_subscription_order_id": o.Order.OrderId, "lmm_price_id": o.Price.ProviderPriceId, "lmm_merchant": o.Merchant}
}
func creemBillingPeriod(p *pb.CreemSubscriptionPrice) string {
	switch {
	case p.Interval == "month" && p.IntervalCount == 1:
		return "every-month"
	case p.Interval == "month" && p.IntervalCount == 3:
		return "every-three-months"
	case p.Interval == "month" && p.IntervalCount == 6:
		return "every-six-months"
	case p.Interval == "year" && p.IntervalCount == 1:
		return "every-year"
	}
	return ""
}
func (a *Creem) validateSubscriptionPrice(ctx context.Context, o *pb.CreemSubscriptionOrder) error {
	if !validCreemOrder(o) || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment || creemBillingPeriod(o.Price) == "" {
		return ErrConflict
	}
	raw, err := a.api(ctx, http.MethodGet, "products?product_id="+url.QueryEscape(o.Price.ProviderPriceId), nil)
	if err != nil {
		return err
	}
	var p struct {
		ID, Object, Mode, Status, Currency string
		Price                              int64
		BillingType                        string `json:"billing_type"`
		BillingPeriod                      string `json:"billing_period"`
		TaxMode                            string `json:"tax_mode"`
	}
	if jsonObject(raw, &p) != nil || p.ID != o.Price.ProviderPriceId || p.Object != "product" || p.Mode != a.mode() || p.Status != "active" || !strings.EqualFold(p.Currency, o.Price.Currency) || strconv.FormatInt(p.Price, 10) != o.Price.AmountMinor || p.BillingType != "recurring" || p.BillingPeriod != creemBillingPeriod(o.Price) || p.TaxMode != "inclusive" {
		return ErrConflict
	}
	return nil
}
func (a *Creem) createSubscriptionCheckout(ctx context.Context, o *pb.CreemSubscriptionOrder) (*CreemHostedCheckout, error) {
	if err := a.validateSubscriptionPrice(ctx, o); err != nil {
		return nil, err
	}
	if _, err := secureURL(o.Price.SuccessUrl); err != nil {
		return nil, err
	}
	raw, err := a.api(ctx, http.MethodPost, "checkouts", map[string]any{"product_id": o.Price.ProviderPriceId, "request_id": o.Order.OrderId, "units": 1, "success_url": o.Price.SuccessUrl, "metadata": creemMetadata(o)})
	if err != nil {
		return nil, err
	}
	return a.subscriptionCheckout(raw, o)
}
func (a *Creem) subscriptionCheckout(raw []byte, o *pb.CreemSubscriptionOrder) (*CreemHostedCheckout, error) {
	var v creemCheckout
	if jsonObject(raw, &v) != nil || !validID(v.ID) || !strings.HasPrefix(v.ID, "ch_") || v.Object != "checkout" || v.Mode != a.mode() || v.RequestID != o.Order.OrderId || refID(v.Product) != o.Price.ProviderPriceId {
		return nil, ErrConflict
	}
	for k, value := range creemMetadata(o) {
		if v.Metadata[k] != value {
			return nil, ErrConflict
		}
	}
	u, err := secureURL(v.URL)
	if err != nil || (u.Hostname() != "checkout.creem.io" && u.Hostname() != "creem.io" && u.Hostname() != "www.creem.io" && u.Hostname() != "test-checkout.creem.io") {
		return nil, ErrConflict
	}
	return &CreemHostedCheckout{ID: v.ID, URL: v.URL}, nil
}
func (a *Creem) findSubscriptionCheckout(ctx context.Context, o *pb.CreemSubscriptionOrder, id string, _ int64) (*CreemHostedCheckout, error) {
	if !validID(id) || !strings.HasPrefix(id, "ch_") {
		return nil, ErrUnavailable
	}
	raw, err := a.api(ctx, http.MethodGet, "checkouts?checkout_id="+url.QueryEscape(id), nil)
	if err != nil {
		return nil, err
	}
	return a.subscriptionCheckout(raw, o)
}

type creemCandidate struct {
	ID, Order, Event   string
	Channel            int64
	Payload, Signature []byte
}

func creemCallbackIdentity(raw []byte) (order, event, subscription string, err error) {
	var e struct {
		ID     string
		Type   string `json:"eventType"`
		Object struct {
			ID           string
			Metadata     map[string]string
			Subscription json.RawMessage
		}
	}
	if jsonObject(raw, &e) != nil || !validID(e.ID) || !strings.HasPrefix(e.ID, "evt_") {
		return "", "", "", ErrInvalid
	}
	order = e.Object.Metadata["lmm_subscription_order_id"]
	subscription = e.Object.ID
	if e.Type == "checkout.completed" {
		subscription = refID(e.Object.Subscription)
	} else if !strings.HasPrefix(e.Type, "subscription.") {
		return "", "", "", ErrUnsupported
	}
	if order != "" && !subscriptionOrderID.MatchString(order) {
		return "", "", "", ErrInvalid
	}
	if !validID(subscription) || !strings.HasPrefix(subscription, "sub_") {
		return "", "", "", ErrInvalid
	}
	return order, e.ID, subscription, nil
}
func (s *CreemSubscriptions) Accept(ctx context.Context, channel int64, r *http.Request) (*pb.CreemSubscriptionEvent, error) {
	if s.adapters[channel] == nil {
		return nil, ErrNotFound
	}
	raw, sig, err := callbackBytes(r, "Creem-Signature")
	if err != nil {
		return nil, err
	}
	order, event, subscription, err := creemCallbackIdentity(raw)
	if err != nil {
		return nil, err
	}
	if order == "" {
		resolved, e := s.core.ResolveCreemSubscriptionOrder(ctx, &pb.CreemSubscriptionLookup{ChannelId: channel, ProviderSubscriptionId: subscription})
		if e != nil {
			return nil, e
		}
		if resolved == nil || !subscriptionOrderID.MatchString(resolved.OrderId) {
			return nil, ErrConflict
		}
		order = resolved.OrderId
	}
	c := creemCandidate{ID: stableKey("creem-subscription-evidence", strconv.FormatInt(channel, 10), order, event, eventHash(raw), eventHash(sig)), Order: order, Event: event, Channel: channel, Payload: raw, Signature: sig}
	if _, err = s.exec(ctx, `INSERT INTO payment_creem_subscription_inbox(evidence_id,order_id,channel_id,event_id,payload,signature) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING`, c.ID, c.Order, c.Channel, c.Event, c.Payload, c.Signature); err != nil {
		return nil, storageError(err)
	}
	return s.settle(ctx, c)
}
func (s *CreemSubscriptions) settle(ctx context.Context, c creemCandidate) (*pb.CreemSubscriptionEvent, error) {
	order, err := s.core.ResolveCreemSubscriptionOrder(ctx, &pb.CreemSubscriptionLookup{ChannelId: c.Channel, ProviderSubscriptionId: creemSubscriptionID(c.Payload)})
	if err == nil && (order == nil || order.OrderId != c.Order) {
		return nil, ErrConflict
	}
	// For first events the binding is not installed yet. Core independently
	// enforces channel_id in the event request as well as merchant/environment.

	digest := sha256.Sum256(c.Payload)
	receipt, err := s.core.GetCreemSubscriptionEvent(ctx, &pb.GetCreemSubscriptionEventRequest{OrderId: c.Order, ProviderEventId: c.Event, PayloadSha256: digest[:], PaymentChannelId: c.Channel})
	if status.Code(err) == codes.NotFound {
		receipt, err = s.core.AcceptCreemSubscriptionEvent(ctx, &pb.AcceptCreemSubscriptionEventRequest{OrderId: c.Order, SignedPayload: c.Payload, Signature: c.Signature, PaymentChannelId: c.Channel})
		if status.Code(err) == codes.InvalidArgument {
			err = fmt.Errorf("%w: original Creem proof", ErrEvidence)
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
		_, err = s.exec(ctx, `UPDATE payment_creem_subscription_inbox SET state='accepted',result=$2,error_code='' WHERE evidence_id=$1 AND payload=$3`, c.ID, encoded, c.Payload)
		if err != nil {
			return nil, storageError(err)
		}
		return receipt, nil
	}
	state := "pending"
	if errors.Is(err, ErrEvidence) {
		state = "rejected"
	}
	_, saveErr := s.exec(ctx, `UPDATE payment_creem_subscription_inbox SET state=$2,attempts=least(attempts+1,1000000),next_attempt=clock_timestamp()+(least(attempts,59)+1)*interval '1 second',error_code=$3 WHERE evidence_id=$1 AND state<>'accepted'`, c.ID, state, errorCode(err))
	if saveErr != nil {
		return nil, storageError(saveErr)
	}
	return nil, err
}
func (s *CreemSubscriptions) Recover(ctx context.Context, limit int) error {
	if limit < 1 || limit > 100 {
		return ErrInvalid
	}
	dbCtx, dbCancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(dbCtx, `SELECT evidence_id,order_id,channel_id,event_id,payload,signature FROM payment_creem_subscription_inbox WHERE state='pending' AND next_attempt<=clock_timestamp() ORDER BY next_attempt,evidence_id LIMIT $1`, limit)
	if err != nil {
		dbCancel()
		return storageError(err)
	}
	var candidates []creemCandidate
	for rows.Next() {
		var c creemCandidate
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

func (s *CreemSubscriptions) recoverCheckouts(ctx context.Context, limit int) error {
	dbCtx, cancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(dbCtx, `SELECT order_id,frozen_sha256,frozen_order,first_dispatch FROM payment_creem_subscription_checkouts WHERE checkout_id IS NULL AND next_lookup<=clock_timestamp() ORDER BY next_lookup,order_id LIMIT $1`, limit)
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
		var o pb.CreemSubscriptionOrder
		err = proto.Unmarshal(p.raw, &o)
		if err == nil {
			var hash string
			hash, err = creemFrozenHash(&o)
			if hash != p.hash || o.Order.OrderId != p.id {
				err = ErrConflict
			}
		}
		if err == nil {
			a := s.adapters[o.Price.ChannelId]
			if a == nil || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment {
				err = ErrUnavailable
			} else {
				var c *CreemHostedCheckout
				c, err = a.findSubscriptionCheckout(ctx, &o, s.recoveryCheckoutID(ctx, p.id), p.first.Unix())
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
			_, saveErr := s.exec(ctx, `UPDATE payment_creem_subscription_checkouts SET lookup_attempts=least(lookup_attempts+1,1000000),next_lookup=clock_timestamp()+(least(lookup_attempts,59)+1)*interval '1 second' WHERE order_id=$1 AND checkout_id IS NULL`, p.id)
			result = errors.Join(result, err, storageError(saveErr))
		}
	}
	return result
}
func (s *CreemSubscriptions) CancelRenewal(ctx context.Context, user, id string) error {
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
	// Cancellation affects renewal only. A provider response never grants or
	// revokes Core entitlements; the original lifecycle callback owns state.
	_, err = a.api(ctx, http.MethodPost, "subscriptions/"+url.PathEscape(o.ProviderSubscriptionId)+"/cancel", map[string]string{"mode": "scheduled", "onExecute": "cancel"})
	return err
}
func creemSubscriptionID(raw []byte) string { _, _, id, _ := creemCallbackIdentity(raw); return id }
func (s *CreemSubscriptions) recoveryCheckoutID(ctx context.Context, id string) string {
	var raw []byte
	if s.scan(ctx, `SELECT payload FROM payment_creem_subscription_inbox WHERE order_id=$1 AND state='accepted' ORDER BY evidence_id LIMIT 1`, []any{id}, &raw) != nil {
		return ""
	}
	var e struct {
		Type   string `json:"eventType"`
		Object struct{ ID string }
	}
	if jsonObject(raw, &e) != nil || e.Type != "checkout.completed" {
		return ""
	}
	return e.Object.ID
}
