package payments

import (
	"testing"
)

func TestCheckoutCatalogueIndependentRuntimeSwitchesAndNoDispatch(t *testing.T) {
	subscriptions, core, inbox := epaySubscriptionRecoveryFixture(t)
	adapter := subscriptions.adapters[2]
	wallet, err := New(newMemory(), newCore(adapter.Channel()), adapter)
	if err != nil {
		t.Fatal(err)
	}
	if got := CheckoutCatalogue(wallet, subscriptions, nil, nil, nil); len(got) != 0 {
		t.Fatal(got)
	}
	subscriptions.newPurchases[2] = true
	got := CheckoutCatalogue(wallet, subscriptions, nil, nil, nil)
	if len(got) != 1 || !got[0].Subscription {
		t.Fatal(got)
	}
	enabled := *adapter
	enabled.cfg.Channel.Enabled = true
	wallet, err = New(newMemory(), newCore(enabled.Channel()), &enabled)
	if err != nil {
		t.Fatal(err)
	}
	got = CheckoutCatalogue(wallet, subscriptions, nil, nil, nil)
	if len(got) != 2 || got[0].Subscription || !got[1].Subscription || got[0].Binding == got[1].Binding {
		t.Fatal(got)
	}
	if core.prepares != 0 || core.confirms != 0 || core.grants != 0 || core.accepted != 0 || inbox.entry.ID != "" || inbox.retries != 0 {
		t.Fatal("read mutated payment state")
	}
}
func TestCheckoutCatalogueFollowsActualPancakeReload(t *testing.T) {
	old := &Pancake{cfg: PancakeConfig{Channel: Channel{ID: 1, Provider: "waffo_pancake", Merchant: "merchant", Environment: "test", Currency: "USD", Enabled: true}, StoreID: "store", ProductID: "old"}}
	next := *old
	next.cfg.ProductID = "next"
	next.cfg.Channel.Merchant = "merchant"
	wallet := &Service{adapters: map[int64]Adapter{1: old}}
	subscriptions := &PancakeSubscriptions{adapters: map[int64]*Pancake{1: old}, admission: map[int64]bool{1: true}}
	config := AttachPancakeConfiguration(wallet, subscriptions)
	before := CheckoutCatalogue(wallet, nil, nil, nil, subscriptions)
	if err := config.Update(func() ([]Adapter, map[int64]bool, error) { return []Adapter{&next}, map[int64]bool{1: false}, nil }, func() error { return nil }); err != nil {
		t.Fatal(err)
	}
	after := CheckoutCatalogue(wallet, nil, nil, nil, subscriptions)
	if len(before) != 2 || len(after) != 1 || after[0].Subscription || before[0].Binding == after[0].Binding {
		t.Fatal(before, after)
	}
}
func TestCheckoutCatalogueBindingMatchesCoreGolden(t *testing.T) {
	c := Channel{ID: 7, Provider: "epay", Merchant: "1001", Environment: "test", Currency: "CNY"}
	if v := checkoutBinding(c, false, "alipay", "", ""); v != "fecbd0c415edfd7f3aaea21e9a980260e914b1671535e2bfaaabed7f5406c016" {
		t.Fatal(v)
	}
	if v := checkoutBinding(c, true, "alipay", "", ""); v != "3725b08cc80a10a1d13d3427ce505106a408133638600318d38097e32f52aebb" {
		t.Fatal(v)
	}
}
