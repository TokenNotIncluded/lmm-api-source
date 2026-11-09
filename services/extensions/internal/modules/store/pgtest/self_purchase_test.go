package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// This is a synthetic receipt port over real Go/PostgreSQL order persistence.
// The actual same-wallet ledger is verified separately in Core's PostgreSQL test.
type selfPurchasePort struct{ *recoveryPort }

func (*selfPurchasePort) SelfPurchaseAvailable() bool { return true }

func TestPostgresSellerSelfPurchaseAccessDeliveryAndRefund(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	port := &recoveryPort{receipts: map[string]store.Receipt{}, losePay: true, loseCapture: true, checkDelivery: func(c store.MoneyCommand) {
		if c.Buyer != c.Seller || c.Total.Amount != 500000 {
			t.Fatal("self delivery changed original parties or price")
		}
	}}
	s, err := store.New(repo, legacyAuthority{}, &selfPurchasePort{port})
	must(t, err)
	ctx := context.Background()
	body := func(v any) json.RawMessage { raw, err := json.Marshal(v); must(t, err); return raw }
	request := func(token, action, id, child string, input any) (any, error) {
		var raw json.RawMessage
		if input != nil {
			raw = body(input)
		}
		return s.Legacy(ctx, token, store.Account{Kind: "personal", ID: map[string]int64{"seller": 1, "buyer": 2, "admin": 4}[token]}, store.LegacyRequest{Action: action, ID: id, ChildID: child, Body: raw, Limit: 100})
	}
	call := func(token, action, id, child string, input any) map[string]any {
		t.Helper()
		value, err := request(token, action, id, child, input)
		must(t, err)
		if value == nil {
			return nil
		}
		return value.(map[string]any)
	}
	terms := call("seller", "save_terms", "", "", map[string]any{"content": "Self-purchase uses the ordinary seller terms", "expected_version": ""})
	call("seller", "accept_disclaimer", "", "", map[string]any{"version": "merchant-store-v1", "accepted": true})
	product := call("seller", "save_product", "", "", map[string]any{"title": "Private self purchase", "visibility": "private", "price_quota": 500000, "template": "fixed-content", "fixed_content": "Private delivered content", "payment_methods": []string{"balance"}, "pickup_login_required": true, "max_quantity_per_order": 1})
	id, variant := product["id"].(string), product["default_variant_id"].(string)
	_, err = db.Exec(`UPDATE lmm_store.variants SET body=jsonb_set(jsonb_set(body,'{track_stock}','true'),'{stock}','2') WHERE id=$1`, variant)
	must(t, err)
	checkout := map[string]any{"product_id": id, "quantity": 1, "payment_method": "balance", "request_key": "private-self", "disclaimer_version": "merchant-store-v1", "seller_terms_version": terms["version"], "accept_seller_terms": true}
	old, err := store.New(repo, legacyAuthority{}, port)
	must(t, err)
	if _, err := old.Legacy(ctx, "seller", store.Account{Kind: "personal", ID: 1}, store.LegacyRequest{Action: "create_order", Body: body(checkout)}); !errors.Is(err, store.ErrPaymentUnavailable) {
		t.Fatalf("old Core self-purchase was not closed: %v", err)
	}
	preview := call("seller", "preview_product", id, "", nil)
	if preview["status"] != "draft" || preview["trading_paused"] != false {
		t.Fatal("private owner cannot buy its active draft", preview)
	}
	if _, err := s.ProductForBuyer(ctx, "buyer", 2, id); !errors.Is(err, store.ErrNotFound) {
		t.Fatal("nonowner saw private product", err)
	}
	checkout["accept_seller_terms"] = false
	if _, err := request("seller", "create_order", "", "", checkout); !errors.Is(err, store.ErrSellerTermsRequired) {
		t.Fatal("self-purchase bypassed seller terms", err)
	}
	checkout["accept_seller_terms"], checkout["quantity"] = true, 2
	if _, err := request("seller", "create_order", "", "", checkout); !errors.Is(err, store.ErrPurchaseLimit) {
		t.Fatal("self-purchase bypassed per-order quantity", err)
	}
	checkout["quantity"] = 1
	created := call("seller", "create_order", "", "", checkout)
	orderID := created["order"].(map[string]any)["id"].(string)
	if created["created"] != true || call("seller", "create_order", "", "", checkout)["created"] != false {
		t.Fatal("self order retry created another order")
	}
	if _, err := request("seller", "pay_order", orderID, "", map[string]any{}); !errors.Is(err, store.ErrPending) {
		t.Fatal("lost pay ACK did not stay pending", err)
	}
	for range 2 {
		s, err = store.New(repo, legacyAuthority{}, &selfPurchasePort{port})
		must(t, err)
		must(t, s.RecoverDeliveries(ctx))
	}
	row, err := repo.FindOrder(ctx, orderID)
	must(t, err)
	if row.Order.State != store.Settled || row.Order.Buyer != row.Order.Seller || !row.Order.Delivered || port.payEffects != 1 || port.captureEffects != 1 {
		t.Fatalf("self delivery or ACK recovery changed: %+v pay=%d capture=%d", row.Order, port.payEffects, port.captureEffects)
	}
	link := call("seller", "pickup_link", orderID, "", nil)["pickup_url"].(string)
	token := strings.TrimPrefix(link, "/store/claim/")
	_, err = s.Pickup(ctx, "seller", 1, token, "", true)
	must(t, err)
	refund := call("seller", "request_refund", orderID, "", map[string]any{"request_key": "self-refund", "mode": "full", "reason": "Self-purchase refund"})
	port.losePay = true
	if _, err := request("seller", "decide_refund", orderID, refund["id"].(string), map[string]any{"decision": "approve"}); !errors.Is(err, store.ErrPending) {
		t.Fatal("lost refund ACK did not stay pending", err)
	}
	must(t, s.RecoverMoney(ctx))
	row, err = repo.FindOrder(ctx, orderID)
	must(t, err)
	if row.Order.State != store.Refunded || port.payEffects != 2 || port.captureEffects != 1 {
		t.Fatal("refund reconciliation duplicated a command", row.Order.State, port.payEffects, port.captureEffects)
	}
	if _, err := s.Pickup(ctx, "seller", 1, token, "", true); !errors.Is(err, store.ErrConflict) {
		t.Fatal("refunded self-purchase still delivered content", err)
	}
	var stock, reserved, claimed int64
	must(t, db.QueryRow(`SELECT (body->>'stock')::bigint,(body->>'reserved')::bigint,(body->>'claimed')::bigint FROM lmm_store.variants WHERE id=$1`, variant).Scan(&stock, &reserved, &claimed))
	if stock != 1 || reserved != 0 || claimed != 1 {
		t.Fatal("self refund restocked consumed delivery", stock, reserved, claimed)
	}
	for _, status := range []string{"paused", "rejected", "deleted"} {
		_, err = db.Exec(`UPDATE lmm_store.products SET body=jsonb_set(body,'{legacy,status}',to_jsonb($2::text)) WHERE id=$1`, id, status)
		must(t, err)
		checkout["request_key"] = "self-" + status
		if _, err := request("seller", "create_order", "", "", checkout); !errors.Is(err, store.ErrNotFound) {
			t.Fatal("inactive private product became purchasable", status, err)
		}
	}
	for _, visibility := range []string{"public", "registered"} {
		product = call("seller", "save_product", "", "", map[string]any{"title": "Self " + visibility, "visibility": visibility, "price_quota": 500000, "template": "fixed-content", "fixed_content": "Normal self purchase", "payment_methods": []string{"balance"}, "pickup_login_required": true})
		id = product["id"].(string)
		call("seller", "submit_product", id, "", nil)
		call("admin", "review_product", id, "", map[string]any{"approve": true})
		checkout["product_id"], checkout["request_key"] = id, "self-"+visibility
		created = call("seller", "create_order", "", "", checkout)
		call("seller", "cancel_order", created["order"].(map[string]any)["id"].(string), "", nil)
	}
}
