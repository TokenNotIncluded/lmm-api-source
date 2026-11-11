package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/composition"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/support"
)

// Reuse the canonical CLI, schema and ACL test, then import synthetic legacy
// rows through the normal Python producer. Never point this at an application DB.
func TestCommerceLegacyProducerActualPostgres(t *testing.T) {
	dsn := os.Getenv("LMM_COMMERCE_PRODUCER_TEST_ADMIN_URL")
	if dsn == "" {
		t.Skip("requires a dedicated empty synthetic PostgreSQL cluster")
	}
	t.Setenv("LMM_COMMERCE_INSTALL_TEST_ADMIN_URL", dsn)
	TestCommerceInstallCLIActualPostgres(t)
	base, err := url.Parse(dsn)
	if err != nil {
		t.Fatal(err)
	}
	root, err := os.MkdirTemp(filepath.Join(os.Getenv("HOME"), ".local/state/lmm-api"), "store-producer-pg-")
	if err != nil {
		t.Fatal(err)
	}
	t.Log("producer PG evidence", root)
	sourceRoot, err := filepath.Abs("../../../..")
	if err != nil {
		t.Fatal(err)
	}
	command := exec.Command("python3", filepath.Join(sourceRoot, "scripts/test_legacy_extensions_import_pg.py"), "-v")
	command.Env = append(os.Environ(), "LMM_COMMERCE_PRODUCER_FIXTURE_DIR="+root,
		"LMM_COMMERCE_PRODUCER_SOCKET="+os.Getenv("LMM_COMMERCE_PRODUCER_SOCKET"),
		"LMM_COMMERCE_PRODUCER_PORT="+base.Port())
	output, err := command.CombinedOutput()
	if writeErr := os.WriteFile(filepath.Join(root, "python-test.log"), output, 0600); writeErr != nil {
		t.Fatal(writeErr)
	}
	if err != nil {
		t.Fatalf("actual producer execution: %v\n%s", err, output)
	}
	var fixture struct {
		ShopID         string   `json:"shop_id"`
		OrderIDs       []string `json:"order_ids"`
		StockIDs       []string `json:"stock_ids"`
		ProductID      string   `json:"product_id"`
		ConversationID string   `json:"conversation_id"`
		MessageIDs     []string `json:"message_ids"`
	}
	raw, err := os.ReadFile(filepath.Join(root, "fixture.private.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	storage := map[string]composition.StorageConfig{}
	pools := map[string]*sql.DB{}
	for _, component := range []string{"store", "support"} {
		connection := *base
		connection.Path = "/lmm_" + component + "_prod"
		connection.User = url.UserPassword("commerce_"+component+"_runtime", "synthetic-only")
		file := filepath.Join(root, component+"-runtime-url")
		if err = os.WriteFile(file, []byte(connection.String()), 0600); err != nil {
			t.Fatal(err)
		}
		storage[component] = composition.StorageConfig{Component: component, Database: "lmm_" + component + "_prod", URLFile: file}
		pool, openErr := sql.Open("postgres", connection.String())
		if openErr != nil {
			t.Fatal(openErr)
		}
		defer pool.Close()
		pools[component] = pool
	}
	token := filepath.Join(root, "identity-token")
	if err = os.WriteFile(token, []byte(strings.Repeat("a", 64)), 0600); err != nil {
		t.Fatal(err)
	}
	composed, err := composition.New(context.Background(), composition.Config{
		Selected: []string{"store", "support"}, Socket: filepath.Join(root, "unused-synthetic-core.sock"),
		StoreStockKeyFile: filepath.Join(root, "stock-key"),
		TokenFile:         token, HostCredential: []byte(strings.Repeat("b", 64)), Store: storage["store"], Support: storage["support"],
	})
	if err != nil {
		t.Fatal("normal composition readback", err)
	}
	defer composed.Close()
	if !composed.Store.StockReady() {
		t.Fatal("normal composition did not load target stock key")
	}
	ctx := context.Background()
	storeRepo, err := store.NewPostgres(pools["store"])
	if err != nil {
		t.Fatal(err)
	}
	for index, id := range fixture.OrderIDs {
		located, readErr := composed.Store.LocateOrder(ctx, id)
		if readErr != nil {
			t.Fatal("composed order reader", readErr)
		}
		order := located.Order
		if located.ShopID != fixture.ShopID || order.ID != id || order.Buyer != (store.Account{Kind: "personal", ID: 1}) || order.Seller != (store.Account{Kind: "personal", ID: 2}) || order.CreatedBy != 2 || order.State != store.Refunded || order.Total.Amount != 100 {
			t.Fatal("source identity/principal changed in native reader", order)
		}
		if order.LegacyRefund == nil || len(order.LegacyRefund.StockIDs) != 1 || order.LegacyRefund.StockIDs[0] != fixture.StockIDs[index] || len(order.LegacyRefund.SourceItems) == 0 {
			t.Fatal("refund stock evidence lost in native decoder")
		}
		role := "buyer"
		if index == 1 {
			role = "seller"
		}
		if order.LegacyRefund.RequestedRole != role {
			t.Fatal("refund requester role changed")
		}
		// A real native write must preserve the imported RawMessage evidence.
		if err = storeRepo.Within(ctx, fixture.ShopID, func(tx store.Tx) error { return tx.Put("orders", id, order) }); err != nil {
			t.Fatal(err)
		}
		again, readErr := composed.Store.LocateOrder(ctx, id)
		if readErr != nil {
			t.Fatal(readErr)
		}
		serialized, marshalErr := json.Marshal(again.Order)
		if marshalErr != nil {
			t.Fatal(marshalErr)
		}
		var body map[string]json.RawMessage
		if err = json.Unmarshal(serialized, &body); err != nil || !strings.Contains(string(body["legacy_refund"]), fixture.StockIDs[index]) || !strings.Contains(string(body["legacy_refund"]), "source-refund-item-") {
			t.Fatal("native JSON roundtrip lost source items", err)
		}
	}
	customer, err := composed.Support.CustomerFromOrder(ctx, fixture.ShopID, fixture.OrderIDs[0])
	if err != nil || customer.Buyer != (store.Account{Kind: "personal", ID: 1}) {
		t.Fatal("composed support/store directory resolved wrong collision account", err)
	}
	supportRepo, err := support.NewPostgres(pools["support"])
	if err != nil {
		t.Fatal(err)
	}
	if err = supportRepo.Within(ctx, fixture.ShopID, func(tx support.Tx) error {
		var conversation support.Conversation
		if readErr := tx.Get("conversations", fixture.ConversationID, &conversation); readErr != nil {
			return readErr
		}
		if conversation.Buyer.ID != 1 || conversation.Seller.ID != 2 || conversation.OrderID != fixture.OrderIDs[0] || conversation.Sequence != 2 || conversation.BuyerReadID != 1 || conversation.SellerReadID != 2 {
			t.Fatal("native support conversation changed parties or read markers", conversation)
		}
		for index, id := range fixture.MessageIDs {
			var message support.Message
			if readErr := tx.Get("messages", id, &message); readErr != nil {
				return readErr
			}
			if message.AuthorUserID != int64(index+2) || message.AuthorAccount.ID != int64(index+1) || message.AuthorAccount.Kind != "personal" {
				t.Fatal("support author user/account confusion", message)
			}
			serialized, marshalErr := json.Marshal(message)
			if marshalErr != nil || !strings.Contains(string(serialized), "synthetic ") {
				t.Fatal("native support message JSON", marshalErr)
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	var operations int
	if err = pools["store"].QueryRow("SELECT count(*) FROM lmm_store.money_operations").Scan(&operations); err != nil || operations != 0 {
		t.Fatal("historical import created financial commands", operations, err)
	}
}
