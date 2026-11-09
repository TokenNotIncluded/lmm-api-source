package pgtest

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"sync"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func TestPostgresCollectionsOfflineUpgrade(t *testing.T) {
	db := legacyDatabase(t)
	_, err := db.Exec(`INSERT INTO lmm_store.shops(shop_id,id,body) VALUES('existing','existing','{"owner":{"kind":"personal","id":1}}'); DROP TABLE lmm_store.cart_items,lmm_store.favorites`)
	must(t, err)
	migration, err := os.ReadFile("../migrations/006_account_collections.sql")
	must(t, err)
	_, err = db.Exec(string(migration))
	must(t, err)
	var existing int
	must(t, db.QueryRow(`SELECT count(*) FROM lmm_store.shops WHERE id='existing'`).Scan(&existing))
	if existing != 1 {
		t.Fatal("collection upgrade changed existing store records")
	}
	_, err = db.Exec(`CREATE ROLE lmm_collection_runtime_test NOLOGIN;
GRANT USAGE ON SCHEMA lmm_store TO lmm_collection_runtime_test;
GRANT SELECT,INSERT,UPDATE,DELETE ON lmm_store.cart_items,lmm_store.favorites TO lmm_collection_runtime_test`)
	must(t, err)
	t.Cleanup(func() {
		_, _ = db.Exec(`DROP OWNED BY lmm_collection_runtime_test; DROP ROLE lmm_collection_runtime_test`)
	})
	ctx := context.Background()
	conn, err := db.Conn(ctx)
	must(t, err)
	defer conn.Close()
	_, err = conn.ExecContext(ctx, `SET ROLE lmm_collection_runtime_test`)
	must(t, err)
	defer conn.ExecContext(ctx, `RESET ROLE`)
	_, err = conn.ExecContext(ctx, `INSERT INTO lmm_store.cart_items(user_id,id,product_id,variant_id,quantity,created_at,updated_at) VALUES(1,repeat('a',64),'product','variant',1,1,1);
INSERT INTO lmm_store.favorites(user_id,product_id,created_at) VALUES(1,'product',1);
UPDATE lmm_store.cart_items SET quantity=2 WHERE user_id=1;
UPDATE lmm_store.favorites SET product_id='product' WHERE user_id=1`)
	must(t, err)
	var quantity int
	must(t, conn.QueryRowContext(ctx, `SELECT quantity FROM lmm_store.cart_items WHERE user_id=1`).Scan(&quantity))
	if quantity != 2 {
		t.Fatal("runtime could not use collection grants")
	}
	_, err = conn.ExecContext(ctx, `DELETE FROM lmm_store.cart_items WHERE user_id=1; DELETE FROM lmm_store.favorites WHERE user_id=1`)
	must(t, err)
	var excessive bool
	must(t, conn.QueryRowContext(ctx, `SELECT has_schema_privilege(current_user,'lmm_store','CREATE') OR has_table_privilege(current_user,'lmm_store.cart_items','TRUNCATE') OR has_table_privilege(current_user,'lmm_store.favorites','TRUNCATE')`).Scan(&excessive))
	if excessive {
		t.Fatal("collection grants acquired installation authority")
	}
}

func TestPostgresAccountCollectionsAndCleanup(t *testing.T) {
	db := legacyDatabase(t)
	repo, err := store.NewPostgres(db)
	must(t, err)
	ledger := &funds{seen: map[string]store.Receipt{}}
	s, err := store.New(repo, legacyAuthority{}, ledger)
	must(t, err)
	ctx := context.Background()
	actors := map[string]int64{"seller": 1, "buyer": 2, "outsider": 3, "admin": 4}
	request := func(token, action, id string, body any) (any, error) {
		raw, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		if body == nil {
			raw = nil
		}
		return s.Legacy(ctx, token, store.Account{Kind: "personal", ID: actors[token]}, store.LegacyRequest{Action: action, ID: id, Body: raw, Limit: 100})
	}
	call := func(token, action, id string, body any) map[string]any {
		t.Helper()
		value, err := request(token, action, id, body)
		must(t, err)
		if value == nil {
			return nil
		}
		raw, err := json.Marshal(value)
		must(t, err)
		var out map[string]any
		must(t, json.Unmarshal(raw, &out))
		return out
	}
	items := func(token, action string) []any {
		t.Helper()
		return call(token, action, "", nil)["items"].([]any)
	}
	call("seller", "save_terms", "", map[string]any{"content": "Collection fixture terms", "expected_version": ""})
	product := call("seller", "save_product", "", map[string]any{"title": "Saved product", "visibility": "public", "price_quota": 500000, "template": "fixed-content", "fixed_content": "Private delivery", "payment_methods": []string{"balance"}, "pickup_login_required": true})
	id := product["id"].(string)
	variant := product["default_variant_id"].(string)
	call("seller", "submit_product", id, nil)
	call("admin", "review_product", id, map[string]any{"approve": true, "note": "Private review"})
	input := map[string]any{"product_id": id, "variant_id": variant, "quantity": 5}
	first := call("buyer", "save_cart", "", input)
	other := call("outsider", "save_cart", "", input)
	for _, token := range []string{"buyer", "outsider"} {
		call(token, "save_favorite", "", map[string]any{"product_id": id})
	}
	if first["id"] == other["id"] {
		t.Fatal("cart identity is not account scoped")
	}
	// Browser guest-cart transfers use absolute quantities, so retried and
	// concurrent transfers must not add the same guest quantity more than once.
	var group sync.WaitGroup
	for range 12 {
		group.Add(1)
		go func() {
			defer group.Done()
			if _, err := request("buyer", "save_cart", "", input); err != nil {
				t.Error(err)
			}
		}()
	}
	group.Wait()
	var count, quantity, reserved, orders int64
	must(t, db.QueryRow(`SELECT count(*),max(quantity) FROM lmm_store.cart_items WHERE user_id=2`).Scan(&count, &quantity))
	must(t, db.QueryRow(`SELECT (body->>'reserved')::bigint FROM lmm_store.variants WHERE id=$1`, variant).Scan(&reserved))
	must(t, db.QueryRow(`SELECT count(*) FROM lmm_store.orders`).Scan(&orders))
	if count != 1 || quantity != 5 || reserved != 0 || orders != 0 || ledger.effects != 0 {
		t.Fatalf("cart write changed quantity/stock/funds: %d %d %d %d %d", count, quantity, reserved, orders, ledger.effects)
	}
	if _, err := s.Legacy(ctx, "buyer", store.Account{Kind: "personal", ID: 3}, store.LegacyRequest{Action: "cart", Limit: 100}); !errors.Is(err, store.ErrForbidden) {
		t.Fatalf("forged collection actor: %v", err)
	}
	call("buyer", "delete_cart", other["id"].(string), nil)
	if len(items("outsider", "cart")) != 1 {
		t.Fatal("cross-account delete removed a cart row")
	}
	call("buyer", "clear_cart", "", nil)
	call("buyer", "clear_favorites", "", nil)
	if len(items("buyer", "cart")) != 0 || len(items("buyer", "favorites")) != 0 || len(items("outsider", "cart")) != 1 || len(items("outsider", "favorites")) != 1 {
		t.Fatal("clear escaped its account")
	}
	call("buyer", "save_cart", "", input)
	call("buyer", "save_favorite", "", map[string]any{"product_id": id})
	recreatedRepo, err := store.NewPostgres(db)
	must(t, err)
	s, err = store.New(recreatedRepo, legacyAuthority{}, ledger)
	must(t, err)
	if len(items("buyer", "cart")) != 1 || len(items("buyer", "favorites")) != 1 {
		t.Fatal("collections were not persisted")
	}
	call("seller", "paused", id, map[string]any{"paused": true})
	cart := items("buyer", "cart")[0].(map[string]any)
	favorite := items("buyer", "favorites")[0].(map[string]any)
	if cart["valid"] != false || cart["product"] == nil || favorite["valid"] != true || favorite["product"] == nil {
		t.Fatal("temporarily paused product was hidden or purchasable")
	}
	cleaned := call("buyer", "cleanup_collections", "", map[string]any{})
	if cleaned["cart_deleted"] != float64(0) || cleaned["favorites_deleted"] != float64(0) {
		t.Fatal("cleanup removed a paused product")
	}
	call("seller", "paused", id, map[string]any{"paused": false})
	_, err = db.Exec(`UPDATE lmm_store.variants SET body=jsonb_set(body,'{legacy,enabled}','false') WHERE id=$1`, variant)
	must(t, err)
	cleaned = call("buyer", "cleanup_collections", "", nil)
	if cleaned["cart_deleted"] != float64(1) || cleaned["favorites_deleted"] != float64(0) {
		t.Fatal("disabled variant cleanup removed the wrong references")
	}
	_, err = db.Exec(`UPDATE lmm_store.variants SET body=jsonb_set(body,'{legacy,enabled}','true') WHERE id=$1`, variant)
	must(t, err)
	call("buyer", "save_cart", "", input)
	_, err = db.Exec(`UPDATE lmm_store.products SET body=jsonb_set(body,'{legacy,visibility}','"private"') WHERE id=$1`, id)
	must(t, err)
	for _, action := range []string{"cart", "favorites"} {
		row := items("buyer", action)[0].(map[string]any)
		if row["valid"] != false || row["product"] != nil || row["unavailable_reason"] != "not_visible" {
			t.Fatalf("private product data leaked in %s: %v", action, row)
		}
	}
	cleaned = call("buyer", "cleanup_collections", "", nil)
	if cleaned["cart_deleted"] != float64(1) || cleaned["favorites_deleted"] != float64(1) {
		t.Fatal("private product references survived cleanup")
	}
	must(t, db.QueryRow(`SELECT count(*) FROM lmm_store.cart_items WHERE user_id=3`).Scan(&count))
	if count != 1 {
		t.Fatal("cleanup changed another account")
	}
}
