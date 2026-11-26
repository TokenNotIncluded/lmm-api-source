package ssmcprelay

import (
	"context"
	"encoding/hex"
	"errors"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

const BytesPerGiB = uint64(1 << 30)
const UnitUSDCents int64 = 100

type Funds interface {
	store.Funds
	CompleteDelivery(context.Context, store.MoneyCommand, string, string) (store.Receipt, error)
}

type Service struct {
	relay  Relay
	repo   store.Repository
	funds  Funds
	seller store.Account
	now    func() time.Time
}

func New(relay Relay, repo store.Repository, funds Funds, seller store.Account) (*Service, error) {
	if store.Missing(relay) || store.Missing(repo) || store.Missing(funds) || !seller.Valid() {
		return nil, store.ErrInvalid
	}
	return &Service{relay, repo, funds, seller, time.Now}, nil
}

// Order contains the immutable quote and the last durably observed business state.
// References are Core receipts. No account or service credentials are persisted.
type Order struct {
	ID                   string        `json:"order_id"`
	UserID               int64         `json:"user_id"`
	Seller               store.Account `json:"seller"`
	Period               string        `json:"period"`
	ResetAt              int64         `json:"reset_at"`
	GiB                  int64         `json:"gib"`
	Bytes                string        `json:"bytes"`
	USDCents             int64         `json:"amount_usd_cents,omitempty"`
	PricingVersion       int           `json:"pricing_version,omitempty"`
	UnitPriceQuotaPerGiB int64         `json:"unit_price_quota_per_gib,omitempty"`
	TotalPriceQuota      int64         `json:"total_price_quota,omitempty"`
	State                string        `json:"state"`
	PaymentRef           string        `json:"payment_ref,omitempty"`
	SettlementRef        string        `json:"settlement_ref,omitempty"`
	RefundRef            string        `json:"refund_ref,omitempty"`
}

func (s *Service) Quota(ctx context.Context, userID int64) (Quota, error) {
	if userID <= 0 {
		return Quota{}, store.ErrUnauthorized
	}
	return s.relay.Quota(ctx, userID)
}

func namespace(user int64) string { return store.ID("ssmcp-relay-orders-v1", user) }
func validID(id string) bool {
	b, e := hex.DecodeString(id)
	return e == nil && len(b) == 32 && hex.EncodeToString(b) == id
}

const maxQuoteCredits int64 = 9007199254740991
const creditsPerUSDCent int64 = 5000

// Old orders keep their original USD MoneyCommand and hash on all retries.
// The added credit fields only describe that existing immutable quote.
func (o *Order) validatePricing() error {
	if o.GiB <= 0 || uint64(o.GiB) > math.MaxUint64/BytesPerGiB || o.Bytes != strconv.FormatUint(uint64(o.GiB)*BytesPerGiB, 10) {
		return store.ErrConflict
	}
	switch o.PricingVersion {
	case 0:
		if o.USDCents != o.GiB*UnitUSDCents {
			return store.ErrConflict
		}
		o.UnitPriceQuotaPerGiB = UnitUSDCents * creditsPerUSDCent
		o.TotalPriceQuota = o.USDCents * creditsPerUSDCent
	case 1:
		unit := o.UnitPriceQuotaPerGiB
		if unit <= 0 || o.GiB > maxQuoteCredits/unit || o.TotalPriceQuota != o.GiB*unit {
			return store.ErrConflict
		}
	default:
		return store.ErrConflict
	}
	return nil
}

func (s *Service) GetOrder(ctx context.Context, user int64, id string) (Order, error) {
	if user <= 0 || !validID(id) {
		return Order{}, store.ErrInvalid
	}
	var o Order
	err := s.repo.Within(ctx, namespace(user), func(tx store.Tx) error {
		if err := tx.Get("commands", id, &o); err != nil {
			return err
		}
		if o.ID != id || o.UserID != user {
			return store.ErrConflict
		}
		return o.validatePricing()
	})
	return o, err
}

func (s *Service) Preview(ctx context.Context, user int64, key string, gib int64) (Order, error) {
	if user <= 0 || strings.TrimSpace(key) == "" || !store.Key(key) || gib <= 0 || uint64(gib) > math.MaxUint64/BytesPerGiB {
		return Order{}, store.ErrInvalid
	}
	id := store.ID("ssmcp-relay-order-v1", user, key)
	existing, err := s.GetOrder(ctx, user, id)
	if err == nil {
		if existing.GiB != gib {
			return Order{}, store.ErrConflict
		}
		return existing, nil
	}
	if !errors.Is(err, store.ErrNotFound) {
		return Order{}, err
	}
	if !s.funds.Available() {
		return Order{}, store.ErrPaymentUnavailable
	}
	q, err := s.Quota(ctx, user)
	if err != nil {
		return Order{}, err
	}
	if q.ResetAt <= s.now().Unix() {
		return Order{}, ErrMonth
	}
	o := Order{ID: id, UserID: user, Seller: s.seller, Period: q.Period, ResetAt: q.ResetAt, GiB: gib, Bytes: strconv.FormatUint(uint64(gib)*BytesPerGiB, 10), PricingVersion: 1, State: "preview"}
	err = s.repo.Within(ctx, namespace(user), func(tx store.Tx) error {
		var previous Order
		switch err := tx.Get("commands", id, &previous); {
		case err == nil:
			if previous.ID != id || previous.UserID != user || previous.GiB != gib {
				return store.ErrConflict
			}
			o = previous
			return o.validatePricing()
		case !errors.Is(err, store.ErrNotFound):
			return err
		default:
			unit, err := tx.RelayPricePerGiB()
			if err != nil {
				return err
			}
			if unit <= 0 || unit > maxQuoteCredits {
				return store.ErrUnavailable
			}
			if gib > maxQuoteCredits/unit {
				return store.ErrInvalid
			}
			o.UnitPriceQuotaPerGiB = unit
			o.TotalPriceQuota = unit * gib
			if o.TotalPriceQuota%creditsPerUSDCent == 0 {
				o.USDCents = o.TotalPriceQuota / creditsPerUSDCent
			}
			return tx.Put("commands", id, o)
		}
	})
	return o, err
}

// advance is compare-and-set under the existing repository's cross-process lock.
// A concurrent replay receives the latest persisted state and proceeds from there.
func (s *Service) advance(ctx context.Context, previous, next Order) (Order, error) {
	var o Order
	err := s.repo.Within(ctx, namespace(previous.UserID), func(tx store.Tx) error {
		if err := tx.Get("commands", previous.ID, &o); err != nil {
			return err
		}
		if o.ID != previous.ID || o.UserID != previous.UserID {
			return store.ErrConflict
		}
		if err := o.validatePricing(); err != nil {
			return err
		}
		if o != previous {
			return nil
		}
		o = next
		return tx.Put("commands", o.ID, o)
	})
	if err != nil {
		return previous, err
	}
	return o, nil
}

func (o Order) entitlement() Entitlement {
	return Entitlement{strconv.FormatInt(o.UserID, 10), o.ID, o.Period, o.Bytes}
}
func (o Order) command(kind string) store.MoneyCommand {
	shop := store.ID("ssmcp-relay-merchant-v1", o.Seller)
	c := store.MoneyCommand{Key: store.ID("store-money-v1", shop, o.ID, kind), Kind: kind, ShopID: shop, OrderID: o.ID,
		Buyer: store.Account{Kind: "personal", ID: o.UserID}, Seller: o.Seller, Total: store.Price{Currency: "USD", Amount: o.USDCents},
		OrderSnapshot: &store.MoneyOrderSnapshot{VariantID: store.ID("ssmcp-relay-gib-v1", o.Period), Quantity: o.GiB, UnitPrice: store.Price{Currency: "USD", Amount: UnitUSDCents}, CreatedBy: o.UserID}}
	if o.PricingVersion == 1 {
		c.Total = store.Price{Amount: o.TotalPriceQuota, Unit: "credits"}
		c.OrderSnapshot.UnitPrice = store.Price{Amount: o.UnitPriceQuotaPerGiB, Unit: "credits"}
	}
	if kind != "pay" {
		c.PaymentRef = o.PaymentRef
	}
	return c
}

func goodReceipt(c store.MoneyCommand, r store.Receipt) bool {
	return r.Key == c.Key && r.CommandHash == store.ID(c) && store.Text(r.Reference, 256) && (r.Outcome == "succeeded" || r.Outcome == "rejected")
}

// Confirm resumes only the immutable order returned by Preview. Core deduplicates
// its fixed command; ssmcpd deduplicates the fixed entitlement across restarts.
func (s *Service) Confirm(ctx context.Context, user int64, token, id string) (Order, error) {
	if token == "" {
		return Order{}, store.ErrUnauthorized
	}
	o, err := s.GetOrder(ctx, user, id)
	if err != nil {
		return Order{}, err
	}
	for {
		next := o
		switch o.State {
		case "completed":
			return o, nil
		case "declined", "refunded":
			return o, store.ErrConflict
		case "expired":
			return o, ErrMonth
		case "refund_required", "needs_review":
			return o, ErrGrantConflict
		case "preview":
			if s.now().Unix() >= o.ResetAt {
				next.State = "expired"
			} else {
				next.State = "payment_pending"
			}
		case "payment_pending":
			cmd := o.command("pay")
			var receipt store.Receipt
			if s.now().Unix() >= o.ResetAt {
				// A lost reply may hide a completed reservation. Never start a new
				// reservation after expiry. Read only the original command's receipt.
				reader, ok := s.funds.(interface {
					Query(context.Context, string, store.MoneyCommand) (store.Receipt, error)
				})
				if !ok {
					return o, store.ErrPending
				}
				receipt, err = reader.Query(ctx, token, cmd)
			} else {
				receipt, err = s.funds.Execute(ctx, token, cmd)
			}
			if err != nil || !goodReceipt(cmd, receipt) {
				return o, store.ErrPending
			}
			if receipt.Outcome == "rejected" {
				next.State = "declined"
			} else {
				next.State = "grant_pending"
				next.PaymentRef = receipt.Reference
			}
		case "grant_pending":
			err = s.relay.Grant(ctx, o.entitlement())
			switch {
			case err == nil:
				next.State = "settlement_pending"
			case errors.Is(err, ErrMonth):
				next.State = "refund_required"
			case errors.Is(err, ErrGrantConflict):
				next.State = "needs_review"
			default:
				return o, store.ErrPending
			}
		case "settlement_pending":
			cmd := o.command("settle")
			receipt, e := s.funds.CompleteDelivery(ctx, cmd, store.ID(o.command("pay")), store.ID("ssmcp-relay-grant-v1", o.entitlement()))
			if e != nil || !goodReceipt(cmd, receipt) {
				return o, store.ErrPending
			}
			if receipt.Outcome == "rejected" {
				next.State = "needs_review"
			} else {
				next.State = "completed"
				next.SettlementRef = receipt.Reference
			}
		default:
			return o, store.ErrConflict
		}
		o, err = s.advance(ctx, o, next)
		if err != nil {
			return o, err
		}
	}
}

// RefundExpired uses the merchant's own Core authority to release a reservation
// only after ssmcpd definitively refused an undelivered, expired order.
func (s *Service) RefundExpired(ctx context.Context, user int64, merchantToken, id string) (Order, error) {
	if merchantToken == "" {
		return Order{}, store.ErrUnauthorized
	}
	o, err := s.GetOrder(ctx, user, id)
	if err != nil {
		return o, err
	}
	if o.State == "refunded" {
		return o, nil
	}
	if o.State != "refund_required" {
		return o, store.ErrConflict
	}
	c := o.command("refund")
	r, err := s.funds.Execute(ctx, merchantToken, c)
	if err != nil || !goodReceipt(c, r) {
		return o, store.ErrPending
	}
	next := o
	if r.Outcome == "rejected" {
		next.State = "needs_review"
	} else {
		next.State = "refunded"
		next.RefundRef = r.Reference
	}
	saved, err := s.advance(ctx, o, next)
	if err == nil && saved.State != "refunded" {
		err = ErrGrantConflict
	}
	return saved, err
}
