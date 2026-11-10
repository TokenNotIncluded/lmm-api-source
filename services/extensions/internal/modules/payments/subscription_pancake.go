package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

//go:embed subscription_pancake_schema.sql
var SubscriptionPancakeSchema string

type PancakeSubscriptionCore interface {
	PreparePancakePurchase(context.Context, string, *pb.PrepareEPayPurchaseRequest) (*pb.PancakeSubscriptionOrder, error)
	ConfirmPancakePurchase(context.Context, string, string) (*pb.PancakeSubscriptionOrder, error)
	ReadPancakePurchase(context.Context, string, string) (*pb.PancakeSubscriptionOrder, error)
	ListPancakePurchases(context.Context, string) (*pb.PancakeSubscriptionOrders, error)
	AcceptPancakeSubscriptionEvent(context.Context, *pb.AcceptPancakeSubscriptionEventRequest) (*pb.PancakeSubscriptionEvent, error)
	GetPancakeSubscriptionEvent(context.Context, *pb.GetPancakeSubscriptionEventRequest) (*pb.PancakeSubscriptionEvent, error)
}
type PancakeHostedCheckout struct {
	ID        string `json:"id"`
	URL       string `json:"url"`
	ExpiresAt int64  `json:"expires_at"`
}
type PancakePurchaseView struct {
	Order    *pb.PancakeSubscriptionOrder
	Checkout *PancakeHostedCheckout
}
type PancakeSubscriptions struct {
	core      PancakeSubscriptionCore
	wallet    *Service
	db        *sql.DB
	adapters  map[int64]*Pancake
	admission map[int64]bool
}

func NewPancakeSubscriptions(core PancakeSubscriptionCore, db *sql.DB, adapters []*Pancake, admission map[int64]bool) (*PancakeSubscriptions, error) {
	if core == nil || db == nil {
		return nil, ErrInvalid
	}
	s := &PancakeSubscriptions{core: core, db: db, adapters: map[int64]*Pancake{}, admission: map[int64]bool{}}
	for _, a := range adapters {
		if a == nil || s.adapters[a.Channel().ID] != nil {
			return nil, ErrInvalid
		}
		s.adapters[a.Channel().ID] = a
		s.admission[a.Channel().ID] = admission[a.Channel().ID]
	}
	ctx, cancel := context.WithTimeout(context.Background(), storageTimeout)
	defer cancel()
	var complete bool
	err := db.QueryRowContext(ctx, `SELECT count(*)=3 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='r' AND c.relname IN('payment_pancake_subscription_checkouts','payment_pancake_subscription_inbox','payment_pancake_wallet_inbox') AND has_schema_privilege(n.oid,'USAGE') AND has_table_privilege(c.oid,'SELECT') AND has_table_privilege(c.oid,'INSERT') AND has_table_privilege(c.oid,'UPDATE')`).Scan(&complete)
	if err != nil || !complete {
		return nil, ErrUnavailable
	}
	return s, nil
}
func (s *PancakeSubscriptions) exec(ctx context.Context, q string, args ...any) (sql.Result, error) {
	bounded, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	return s.db.ExecContext(bounded, q, args...)
}
func (s *PancakeSubscriptions) scan(ctx context.Context, q string, args []any, dest ...any) error {
	bounded, cancel := context.WithTimeout(ctx, storageTimeout)
	defer cancel()
	return s.db.QueryRowContext(bounded, q, args...).Scan(dest...)
}
func validPancakeOrder(o *pb.PancakeSubscriptionOrder) bool {
	if o == nil || o.Order == nil || o.Price == nil {
		return false
	}
	b, p := o.Order, o.Price
	n, err := strconv.ParseInt(p.AmountMinor, 10, 64)
	return err == nil && validAmount(n) && strconv.FormatInt(n, 10) == p.AmountMinor && subscriptionOrderID.MatchString(b.OrderId) && b.AccountId > 0 && len(b.PlanJson) > 0 && b.PlanId == p.PlanId && b.PlanRevision == p.PlanRevision && b.PaymentChannelId == p.ChannelId && b.Currency == p.Currency && b.AmountMinor == p.AmountMinor && (p.Currency == "USD" || p.Currency == "CNY") && (p.ProductType == "one_time" || p.ProductType == "subscription") && validID(p.StoreId) && validID(p.ProductId) && validID(o.ProviderReference)
}
func (s *PancakeSubscriptions) Prepare(ctx context.Context, user, key string, plan, channel int64) (*pb.PancakeSubscriptionOrder, error) {
	if !validKey(key) || plan <= 0 || s.adapters[channel] == nil || !s.admission[channel] {
		return nil, ErrInvalid
	}
	o, err := s.core.PreparePancakePurchase(ctx, user, &pb.PrepareEPayPurchaseRequest{Mutation: &pb.MutationContext{IdempotencyKey: key}, PlanId: plan, PaymentChannelId: channel})
	if err != nil {
		return nil, err
	}
	if !validPancakeOrder(o) || o.Order.PlanId != plan || o.Order.PaymentChannelId != channel {
		return nil, ErrConflict
	}
	return o, nil
}
func (s *PancakeSubscriptions) Read(ctx context.Context, user, id string) (*PancakePurchaseView, error) {
	if !validID(id) {
		return nil, ErrInvalid
	}
	o, err := s.core.ReadPancakePurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validPancakeOrder(o) {
		return nil, ErrConflict
	}
	view := &PancakePurchaseView{Order: o}
	var c PancakeHostedCheckout
	err = s.scan(ctx, `SELECT checkout_id,checkout_url,checkout_expires FROM payment_pancake_subscription_checkouts WHERE order_id=$1 AND checkout_id IS NOT NULL`, []any{o.Order.OrderId}, &c.ID, &c.URL, &c.ExpiresAt)
	if err == nil {
		view.Checkout = &c
	} else if !errors.Is(err, sql.ErrNoRows) {
		return nil, storageError(err)
	}
	return view, nil
}
func (s *PancakeSubscriptions) List(ctx context.Context, user string) (*pb.PancakeSubscriptionOrders, error) {
	return s.core.ListPancakePurchases(ctx, user)
}
func pancakeFrozenHash(o *pb.PancakeSubscriptionOrder) (string, error) {
	if !validPancakeOrder(o) {
		return "", ErrConflict
	}
	price, err := (proto.MarshalOptions{Deterministic: true}).Marshal(o.Price)
	if err != nil {
		return "", ErrInvalid
	}
	return stableKey(o.Order.OrderId, o.ProviderReference, strconv.FormatInt(o.Order.AccountId, 10), string(o.Order.PlanJson), string(price), o.Merchant, o.Environment)[4:], nil
}
func (s *PancakeSubscriptions) Confirm(ctx context.Context, user, id string) (*PancakePurchaseView, error) {
	prior, err := s.core.ReadPancakePurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validPancakeOrder(prior) {
		return nil, ErrConflict
	}
	if prior.Order.ConfirmedAtUnix == nil && !s.admission[prior.Order.PaymentChannelId] {
		return nil, ErrDenied
	}
	o, err := s.core.ConfirmPancakePurchase(ctx, user, id)
	if err != nil {
		return nil, err
	}
	if !validPancakeOrder(o) || o.Order.OrderId != prior.Order.OrderId || o.Order.ConfirmedAtUnix == nil {
		return nil, ErrConflict
	}
	// Imported originals never create a replacement provider checkout. The user
	// can always read their original pending order; paid raw events remain live.
	if len(o.LegacySourceJson) > 0 {
		return s.Read(ctx, user, o.Order.OrderId)
	}
	a := s.adapters[o.Order.PaymentChannelId]
	if a == nil || a.cfg.Channel.Merchant != o.Merchant || a.cfg.Channel.Environment != o.Environment || a.cfg.Channel.Currency != o.Price.Currency {
		return nil, ErrUnavailable
	}
	h, err := pancakeFrozenHash(o)
	if err != nil {
		return nil, err
	}
	raw, err := proto.Marshal(o)
	if err != nil {
		return nil, ErrInvalid
	}
	if _, err = s.exec(ctx, `INSERT INTO payment_pancake_subscription_checkouts(order_id,channel_id,frozen_sha256,frozen_order) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING`, o.Order.OrderId, o.Order.PaymentChannelId, h, raw); err != nil {
		return nil, storageError(err)
	}
	var saved string
	var first time.Time
	var checkout sql.NullString
	if err = s.scan(ctx, `SELECT frozen_sha256,first_dispatch,checkout_id FROM payment_pancake_subscription_checkouts WHERE order_id=$1`, []any{o.Order.OrderId}, &saved, &first, &checkout); err != nil {
		return nil, storageError(err)
	}
	if saved != h {
		return nil, ErrConflict
	}
	if checkout.Valid {
		return s.Read(ctx, user, o.Order.OrderId)
	}
	if time.Since(first) >= a.ReplayWindow("checkout") {
		return nil, ErrUnavailable
	}
	c, err := a.createSubscriptionCheckout(ctx, o)
	if err != nil {
		return nil, err
	}
	if err = s.saveCheckout(ctx, o.Order.OrderId, c); err != nil {
		return nil, err
	}
	return s.Read(ctx, user, o.Order.OrderId)
}
func (s *PancakeSubscriptions) saveCheckout(ctx context.Context, id string, c PancakeHostedCheckout) error {
	var saved string
	err := s.scan(ctx, `UPDATE payment_pancake_subscription_checkouts SET checkout_id=$2,checkout_url=$3,checkout_expires=$4,recovered_at=clock_timestamp() WHERE order_id=$1 AND (checkout_id IS NULL OR (checkout_id=$2 AND checkout_url=$3 AND checkout_expires=$4)) RETURNING checkout_id`, []any{id, c.ID, c.URL, c.ExpiresAt}, &saved)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrConflict
	}
	if err != nil {
		return storageError(err)
	}
	return nil
}
func (p *Pancake) createSubscriptionCheckout(ctx context.Context, o *pb.PancakeSubscriptionOrder) (PancakeHostedCheckout, error) {
	if !validPancakeOrder(o) || p.cfg.Channel.Merchant != o.Merchant || p.cfg.Channel.Environment != o.Environment {
		return PancakeHostedCheckout{}, ErrConflict
	}
	frozen := o.Price
	// Reuse the pinned old catalog query; per-session price snapshots apply to
	// both one-time and recurring products, as in go-v0.2.118.
	result, err := p.client.GraphQL.Query(ctx, pancake.GraphQLParams{Query: `query ($storeId: String!) { onetimeProducts(storeId: $storeId, filter: { status: { eq: "active" } }) { id status } subscriptionProducts(storeId: $storeId, filter: { status: { eq: "active" } }) { id status billingPeriod } }`, Variables: map[string]any{"storeId": frozen.StoreId}})
	if err != nil || result == nil || len(result.Errors) > 0 {
		return PancakeHostedCheckout{}, ErrUnavailable
	}
	type product struct{ ID, Status, BillingPeriod string }
	var data struct {
		OnetimeProducts      []product
		SubscriptionProducts []product
	}
	if jsonObject(result.Data, &data) != nil {
		return PancakeHostedCheckout{}, ErrConflict
	}
	products := data.OnetimeProducts
	if frozen.ProductType == "subscription" {
		products = data.SubscriptionProducts
	}
	found := false
	for _, v := range products {
		if v.ID == frozen.ProductId && v.Status == "active" && (frozen.ProductType == "one_time" || v.BillingPeriod == frozen.BillingPeriod) {
			found = true
		}
	}
	if !found {
		return PancakeHostedCheckout{}, ErrConflict
	}
	if _, err = secureURL(frozen.ReturnUrl); err != nil {
		return PancakeHostedCheckout{}, ErrConflict
	}
	n, err := strconv.ParseInt(frozen.AmountMinor, 10, 64)
	if err != nil {
		return PancakeHostedCheckout{}, ErrConflict
	}
	expires := 45 * 60
	params := pancake.CreateCheckoutSessionParams{ProductID: frozen.ProductId, Currency: frozen.Currency, PriceSnapshot: &pancake.PriceSnapshot{Amount: decimalMinor(n), TaxCategory: pancake.TaxCategory(frozen.TaxCategory)}, OrderMerchantExternalID: &o.ProviderReference, SuccessURL: &frozen.ReturnUrl, ExpiresInSeconds: &expires, Metadata: map[string]string{"lmm_product_id": frozen.ProductId, "lmm_plan_id": frozen.ProviderPlanReference}}
	if frozen.CheckoutRegion == "china" {
		params.BillingDetail = &pancake.BillingDetail{Country: "CN", IsBusiness: false}
	}
	if frozen.CheckoutLanguage != "" {
		language := pancake.CashierLanguage(frozen.CheckoutLanguage)
		params.Language = &language
	}
	// An ordinary session URL carries no customer-login token to persist/cache.
	session, err := p.client.Checkout.Anonymous.Create(ctx, params, pancake.WithIdempotencyKey(stableKey("pancake-subscription-checkout", o.Order.OrderId)))
	if err != nil {
		return PancakeHostedCheckout{}, ErrUnavailable
	}
	if session == nil || !validID(session.SessionID) {
		return PancakeHostedCheckout{}, ErrConflict
	}
	if _, err = secureURL(session.CheckoutURL); err != nil {
		return PancakeHostedCheckout{}, ErrConflict
	}
	expiry, err := time.Parse(time.RFC3339, session.ExpiresAt)
	if err != nil {
		return PancakeHostedCheckout{}, ErrConflict
	}
	return PancakeHostedCheckout{ID: session.SessionID, URL: session.CheckoutURL, ExpiresAt: expiry.Unix()}, nil
}

type pancakeCandidate struct {
	ID, Reference, Event, ProviderOrder string
	Channel                             int64
	Payload, Signature                  []byte
}

func pancakeCallback(raw []byte) (pancakeCandidate, error) {
	var v struct {
		ID, EventID string
		Data        struct {
			Reference string `json:"orderMerchantExternalId"`
			OrderID   string `json:"orderId"`
		}
	}
	if jsonObject(raw, &v) != nil {
		return pancakeCandidate{}, ErrInvalid
	}
	event := v.ID
	if event == "" {
		event = v.EventID
	}
	ref := v.Data.Reference
	if ref == "" {
		ref = v.Data.OrderID
	}
	if !validID(ref) || !validID(event) || !validID(v.Data.OrderID) {
		return pancakeCandidate{}, ErrInvalid
	}
	return pancakeCandidate{Reference: ref, Event: event, ProviderOrder: v.Data.OrderID}, nil
}
func (s *PancakeSubscriptions) Accept(ctx context.Context, channel int64, r *http.Request) (*pb.PancakeSubscriptionEvent, error) {
	if channel < 0 || (channel > 0 && s.adapters[channel] == nil) {
		return nil, ErrNotFound
	}
	raw, sig, err := callbackBytes(r, "X-Waffo-Signature")
	if err != nil {
		return nil, err
	}
	c, err := pancakeCallback(raw)
	if err != nil {
		return nil, err
	}
	c.Channel = channel
	c.Payload = raw
	c.Signature = sig
	c.ID = stableKey("pancake-subscription-evidence", strconv.FormatInt(channel, 10), c.Reference, c.Event, eventHash(raw), eventHash(sig))
	if _, err = s.exec(ctx, `INSERT INTO payment_pancake_subscription_inbox(evidence_id,provider_reference,channel_id,event_id,payload,signature) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING`, c.ID, c.Reference, channel, c.Event, raw, sig); err != nil {
		return nil, storageError(err)
	}
	return s.settlePancake(ctx, c)
}
func (s *PancakeSubscriptions) settlePancake(ctx context.Context, c pancakeCandidate) (*pb.PancakeSubscriptionEvent, error) {
	digest := sha256.Sum256(c.Payload)
	receipt, err := s.core.GetPancakeSubscriptionEvent(ctx, &pb.GetPancakeSubscriptionEventRequest{PaymentChannelId: c.Channel, ProviderReference: c.Reference, ProviderEventId: c.Event, PayloadSha256: digest[:]})
	if status.Code(err) == codes.NotFound {
		receipt, err = s.core.AcceptPancakeSubscriptionEvent(ctx, &pb.AcceptPancakeSubscriptionEventRequest{PaymentChannelId: c.Channel, ProviderReference: c.Reference, SignedPayload: c.Payload, Signature: c.Signature})
		if status.Code(err) == codes.InvalidArgument {
			err = fmt.Errorf("%w: invalid original Pancake proof", ErrEvidence)
		}
	}
	if err == nil && (receipt == nil || receipt.EventId != c.Event || !subscriptionOrderID.MatchString(receipt.OrderId) || receipt.ProviderOrderId != c.ProviderOrder || !bytes.Equal(receipt.PayloadSha256, digest[:])) {
		err = ErrConflict
	}
	if err == nil {
		encoded, e := proto.Marshal(receipt)
		if e != nil {
			return nil, ErrUnavailable
		}
		_, e = s.exec(ctx, `UPDATE payment_pancake_subscription_inbox SET state='accepted',result=$2,error_code='' WHERE evidence_id=$1`, c.ID, encoded)
		if e != nil {
			return nil, storageError(e)
		}
		return receipt, nil
	}
	state := "pending"
	if errors.Is(err, ErrEvidence) {
		state = "rejected"
	}
	if _, e := s.exec(ctx, `UPDATE payment_pancake_subscription_inbox SET state=$2,attempts=least(attempts+1,1000000),next_attempt=clock_timestamp()+(least(attempts,59)+1)*interval '1 second',error_code=$3 WHERE evidence_id=$1 AND state<>'accepted'`, c.ID, state, errorCode(err)); e != nil {
		return nil, storageError(e)
	}
	return nil, err
}
func (s *PancakeSubscriptions) Recover(ctx context.Context, limit int) error {
	if limit < 1 || limit > 100 {
		return ErrInvalid
	}
	walletErr := s.recoverWallet(ctx, limit)
	bounded, cancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(bounded, `SELECT evidence_id,provider_reference,channel_id,event_id,payload,signature FROM payment_pancake_subscription_inbox WHERE state='pending' AND next_attempt<=clock_timestamp() ORDER BY next_attempt,evidence_id LIMIT $1`, limit)
	if err != nil {
		cancel()
		return storageError(err)
	}
	var candidates []pancakeCandidate
	for rows.Next() {
		var c pancakeCandidate
		if err = rows.Scan(&c.ID, &c.Reference, &c.Channel, &c.Event, &c.Payload, &c.Signature); err != nil {
			break
		}
		parsed, e := pancakeCallback(c.Payload)
		if e != nil {
			err = e
			break
		}
		c.ProviderOrder = parsed.ProviderOrder
		candidates = append(candidates, c)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	cancel()
	if err != nil {
		return storageError(err)
	}
	var result error
	for _, c := range candidates {
		if _, err = s.settlePancake(ctx, c); err != nil {
			result = errors.Join(result, err)
		}
	}
	return errors.Join(walletErr, result, s.recoverPancakeCheckouts(ctx, limit))
}
func (s *PancakeSubscriptions) recoverPancakeCheckouts(ctx context.Context, limit int) error {
	bounded, cancel := context.WithTimeout(ctx, storageTimeout)
	rows, err := s.db.QueryContext(bounded, `SELECT order_id,frozen_sha256,frozen_order,first_dispatch FROM payment_pancake_subscription_checkouts WHERE checkout_id IS NULL AND next_lookup<=clock_timestamp() ORDER BY next_lookup,order_id LIMIT $1`, limit)
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
			break
		}
		all = append(all, p)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	cancel()
	if err != nil {
		return storageError(err)
	}
	var result error
	for _, p := range all {
		var o pb.PancakeSubscriptionOrder
		err = proto.Unmarshal(p.raw, &o)
		if err == nil {
			h, e := pancakeFrozenHash(&o)
			if e != nil || h != p.hash || o.Order.OrderId != p.id {
				err = ErrConflict
			}
		}
		var c PancakeHostedCheckout
		if err == nil {
			a := s.adapters[o.Order.PaymentChannelId]
			if a == nil || time.Since(p.first) >= a.ReplayWindow("checkout") {
				err = ErrUnavailable
			} else {
				c, err = a.createSubscriptionCheckout(ctx, &o)
			}
		}
		if err == nil {
			err = s.saveCheckout(ctx, p.id, c)
		}
		if err != nil {
			_, e := s.exec(ctx, `UPDATE payment_pancake_subscription_checkouts SET lookup_attempts=least(lookup_attempts+1,1000000),next_lookup=clock_timestamp()+interval '1 minute' WHERE order_id=$1 AND checkout_id IS NULL`, p.id)
			result = errors.Join(result, err)
			if e != nil {
				result = errors.Join(result, storageError(e))
			}
		}
	}
	return result
}
