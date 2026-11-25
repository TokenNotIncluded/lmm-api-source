// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package ssmcpd

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"math"
	"net/http"
	"regexp"
	"strconv"
)

const trafficService = "ssmcpd.monthly-traffic.v1"

type CommerceConfig struct {
	ShopID       string         `toml:"shop_id"`
	VariantID    string         `toml:"variant_id"`
	BytesPerUnit int64          `toml:"bytes_per_unit"`
	Grants       InternalConfig `toml:"grants"`
}
type trafficCommerce struct {
	cfg    CommerceConfig
	shop   *store.Service
	grants *internalClient
}

func (s *Service) ConfigureCommerce(cfg *CommerceConfig, shop *store.Service) error {
	if cfg == nil {
		return nil
	}
	if !s.MeteringConfigured() || shop == nil || !shop.ServiceDeliveryAvailable() || !bearerPattern.MatchString(cfg.ShopID) || !bearerPattern.MatchString(cfg.VariantID) || cfg.BytesPerUnit <= 0 || cfg.Grants.Origin != s.meter.origin {
		return errors.New("ssmcpd commerce requires metering and grants for the same daemon, a Core-backed store, and a configured traffic variant")
	}
	client, err := newInternalClient(cfg.Grants)
	if err != nil {
		return err
	}
	s.commerce = &trafficCommerce{*cfg, shop, client}
	return nil
}
func (s *Service) CommerceConfigured() bool { return s != nil && s.commerce != nil }

type Offer struct {
	Period       string `json:"period"`
	UnitCredits  string `json:"unit_credits"`
	BytesPerUnit string `json:"bytes_per_unit"`
}
type PurchaseRequest struct {
	RequestKey string `json:"request_key"`
	Quantity   int64  `json:"quantity"`
	Offer
}
type OrderStatus struct {
	OrderID     string `json:"order_id"`
	MoneyState  string `json:"money_state"`
	GrantState  string `json:"grant_state"`
	RefundState string `json:"refund_state,omitempty"`
	Period      string `json:"period"`
	Bytes       string `json:"bytes"`
	Credits     string `json:"credits"`
}

var requestKey = regexp.MustCompile("^[a-zA-Z0-9_-]{1,96}$")

func personal(user int64) store.Account { return store.Account{Kind: "personal", ID: user} }
func (s *Service) Offer(ctx context.Context, token string, user int64) (Offer, error) {
	if !s.CommerceConfigured() {
		return Offer{}, store.ErrPaymentUnavailable
	}
	c := s.commerce
	q, err := s.Quota(ctx, user)
	if errors.Is(err, ErrUnbound) {
		return Offer{}, store.ErrNotFound
	}
	if err != nil {
		return Offer{}, store.ErrUnavailable
	}
	price, err := c.shop.ServiceUnitPrice(ctx, token, personal(user), c.cfg.ShopID, c.cfg.VariantID)
	if err != nil {
		return Offer{}, err
	}
	return Offer{q.Period, strconv.FormatInt(price.Amount, 10), strconv.FormatInt(c.cfg.BytesPerUnit, 10)}, nil
}
func projection(o store.Order) OrderStatus {
	t := o.Entitlement.Terms
	state := o.Entitlement.Dispatch
	if state == "" {
		state = "not_dispatched"
		if o.State == store.Settled && o.RefundState != "requested" {
			state = "redeemable"
		}
	}
	return OrderStatus{o.ID, o.State, state, o.RefundState, t.Period, strconv.FormatInt(t.UnitsPerItem*o.Quantity, 10), strconv.FormatInt(o.Total.Amount, 10)}
}
func (c *trafficCommerce) lookup(ctx context.Context, token string, user int64, id string) (string, store.Order, error) {
	row, err := c.shop.LocateOrder(ctx, id)
	if err != nil {
		return "", store.Order{}, err
	}
	o, err := c.shop.ServiceOrder(ctx, token, personal(user), row.ShopID, id, trafficService)
	return row.ShopID, o, err
}
func (s *Service) Order(ctx context.Context, token string, user int64, id string) (OrderStatus, error) {
	if !s.CommerceConfigured() {
		return OrderStatus{}, store.ErrPaymentUnavailable
	}
	if !bearerPattern.MatchString(id) {
		return OrderStatus{}, store.ErrInvalid
	}
	_, o, err := s.commerce.lookup(ctx, token, user, id)
	if err != nil && !(errors.Is(err, store.ErrPending) && o.Entitlement != nil) {
		return OrderStatus{}, err
	}
	return projection(o), nil
}
func (s *Service) Buy(ctx context.Context, token string, user int64, request PurchaseRequest) (OrderStatus, error) {
	if !s.CommerceConfigured() {
		return OrderStatus{}, store.ErrPaymentUnavailable
	}
	c := s.commerce
	buyer := personal(user)
	if !requestKey.MatchString(request.RequestKey) || request.Quantity <= 0 || request.Quantity > math.MaxInt64/c.cfg.BytesPerUnit {
		return OrderStatus{}, store.ErrInvalid
	}
	key := "ssmcpd-" + request.RequestKey
	// Resolve an existing checkout before consulting a new month's quota or a
	// repriced catalogue. Only a still-unpaid original order needs new consent.
	id := store.ID("order", buyer, user, key)
	originalShop, existing, err := c.lookup(ctx, token, user, id)
	if err == nil || (errors.Is(err, store.ErrPending) && existing.Entitlement != nil) {
		t := existing.Entitlement.Terms
		if request.Period != t.Period || request.BytesPerUnit != strconv.FormatInt(t.UnitsPerItem, 10) || request.UnitCredits != strconv.FormatInt(t.UnitPrice.Amount, 10) || request.Quantity != existing.Quantity {
			return OrderStatus{}, store.ErrConflict
		}
		if existing.State == store.AwaitingPayment && (originalShop != c.cfg.ShopID || existing.VariantID != c.cfg.VariantID || t.Audience != store.ID("ssmcpd-origin", c.grants.origin)) {
			return OrderStatus{}, store.ErrConflict
		}
		if existing.State != store.AwaitingPayment {
			return projection(existing), nil
		}
	} else if !errors.Is(err, store.ErrNotFound) {
		return OrderStatus{}, err
	}
	offer, err := s.Offer(ctx, token, user)
	if err != nil {
		return OrderStatus{}, err
	}
	if request.Offer != offer {
		return OrderStatus{}, store.ErrConflict
	}
	amount, _ := strconv.ParseInt(offer.UnitCredits, 10, 64)
	t := store.ServiceTerms{Service: trafficService, Audience: store.ID("ssmcpd-origin", c.grants.origin), Period: offer.Period, UnitsPerItem: c.cfg.BytesPerUnit, UnitPrice: store.Price{Unit: "credits", Amount: amount}}
	o, err := c.shop.PurchaseServiceEntitlement(ctx, token, store.Purchase{ShopID: c.cfg.ShopID, VariantID: c.cfg.VariantID, Buyer: buyer, Quantity: request.Quantity, Key: key}, t)
	if err != nil {
		return OrderStatus{}, err
	}
	if o.State == store.AwaitingPayment {
		paid, err := c.shop.ChangeOrder(ctx, token, store.OrderChange{ShopID: c.cfg.ShopID, OrderID: o.ID, Action: "pay"})
		if err != nil {
			if paid.Entitlement != nil && (errors.Is(err, store.ErrPending) || paid.State == store.Cancelled) {
				return projection(paid), nil
			}
			return OrderStatus{}, err
		}
		o = paid
	}
	return projection(o), nil
}

type grantRequest struct {
	UserID  string `json:"user_id"`
	OrderID string `json:"order_id"`
	Period  string `json:"period"`
	Bytes   string `json:"bytes"`
}
type grantReply struct {
	grantRequest
	Status string `json:"status"`
}

func (s *Service) Redeem(ctx context.Context, token string, user int64, id string) (OrderStatus, error) {
	if !s.CommerceConfigured() {
		return OrderStatus{}, store.ErrPaymentUnavailable
	}
	if !bearerPattern.MatchString(id) {
		return OrderStatus{}, store.ErrInvalid
	}
	c := s.commerce
	b := personal(user)
	shopID, o, err := c.lookup(ctx, token, user, id)
	if err != nil {
		if errors.Is(err, store.ErrPending) && o.Entitlement != nil {
			return projection(o), nil
		}
		return OrderStatus{}, err
	}
	if o.Entitlement.Terms.Audience != store.ID("ssmcpd-origin", c.grants.origin) {
		return OrderStatus{}, store.ErrUnavailable
	}
	if o.RefundRef != "" || o.State == store.Refunded || o.State == store.RefundPending || (o.RefundState == "requested" && o.Entitlement.Dispatch != "unknown") {
		return projection(o), nil
	}
	if o.State == store.Paid || o.State == store.Fulfilled {
		saved, err := c.shop.FinalizeDelivery(ctx, shopID, id)
		if err != nil {
			if errors.Is(err, store.ErrPending) {
				return s.Order(ctx, token, user, id)
			}
			return OrderStatus{}, err
		}
		o = saved
	}
	if o.State != store.Settled || o.Entitlement.Dispatch == "granted" || o.Entitlement.Dispatch == "rejected" {
		return projection(o), nil
	}
	o, err = c.shop.ClaimServiceDispatch(ctx, token, b, shopID, id, trafficService)
	if err != nil {
		return OrderStatus{}, err
	}
	status := projection(o)
	if status.GrantState == "granted" {
		return status, nil
	}
	req := grantRequest{strconv.FormatInt(o.Buyer.ID, 10), o.ID, status.Period, status.Bytes}
	decision := c.grants.issue(ctx, req)
	if decision != "" {
		if err := c.shop.RecordServiceDispatch(ctx, shopID, o, decision == "granted"); err != nil {
			return status, err
		}
		status.GrantState = decision
	}
	return status, nil
}

// A missing or inconsistent ACK leaves the durable original operation unknown.
func (client *internalClient) issue(ctx context.Context, req grantRequest) string {
	code, body, err := client.post(ctx, "/internal/v1/ssmcp/entitlements", req)
	if err != nil {
		return ""
	}
	var receipt grantReply
	if code == http.StatusOK && json.Unmarshal(body, &receipt) == nil && receipt.grantRequest == req && (receipt.Status == "granted" || receipt.Status == "replayed") {
		return "granted"
	}
	if code == http.StatusConflict {
		var failure struct {
			Error string `json:"error"`
		}
		if json.Unmarshal(body, &failure) == nil && (failure.Error == "order_conflict" || failure.Error == "period_mismatch" || failure.Error == "ledger_limit") {
			return "rejected"
		}
	}
	return ""
}

func (s *Service) RequestRefund(ctx context.Context, token string, user int64, id, reason string) (OrderStatus, error) {
	if !s.CommerceConfigured() {
		return OrderStatus{}, store.ErrPaymentUnavailable
	}
	if !bearerPattern.MatchString(id) || !store.Text(reason, 2000) {
		return OrderStatus{}, store.ErrInvalid
	}
	shop, o, err := s.commerce.lookup(ctx, token, user, id)
	if err != nil {
		return OrderStatus{}, err
	}
	if o.State == store.Refunded {
		return projection(o), nil
	}
	o, err = s.commerce.shop.ChangeOrder(ctx, token, store.OrderChange{ShopID: shop, OrderID: id, Action: "request_refund", Text: reason})
	if err != nil {
		return OrderStatus{}, err
	}
	return projection(o), nil
}
