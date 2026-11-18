// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package composition

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

func pancakeManagementBindingsFixture(t *testing.T) (*PancakeAdministration, *payments.Service, *configurationOrders, *configurationQuotes) {
	t.Helper()
	live := multiPaymentChannel{
		ID: 51, Provider: "waffo_pancake", Merchant: "MER_0123456789abcdefghijkl",
		Environment: "live", Currency: "USD", PrivateKeyFile: paymentPrivate(t, "live.pem", adminPrivateKey(t)),
		StoreID: "STO_0123456789abcdefghijkl", ProductID: "PROD_0123456789abcdefghijkl",
		TaxCategory: "saas", ReturnURL: "https://example.test/live",
	}
	test := live
	test.ID, test.Environment = 52, "test"
	test.StoreID, test.ProductID = "STO_abcdefghijkl0123456789", "PROD_abcdefghijkl0123456789"
	test.PrivateKeyFile, test.ReturnURL = paymentPrivate(t, "test.pem", adminPrivateKey(t)), "https://example.test/test"
	cny := live
	cny.ID, cny.Currency = 53, "CNY"
	return pancakeManagementDocumentFixture(t, multiPaymentDocument{Version: 2, Channels: []multiPaymentChannel{live, test, cny}})
}

func TestPancakeManagementListsDisabledBindingsAndRequiresSelection(t *testing.T) {
	admin, wallet, _, quotes := pancakeManagementBindingsFixture(t)
	before := append([]byte(nil), admin.expected...)
	value, err := admin.Configuration(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	view := value.(pancakeConfiguration)
	if view.pancakeConfigurationBinding != nil || len(view.Bindings) != 2 {
		t.Fatal("multiple bindings were collapsed into an implicit selection")
	}
	live, test := view.Bindings[0], view.Bindings[1]
	if live.ChannelID != 51 || live.Environment != "live" || len(live.Channels) != 2 ||
		live.Channels[0].ID != 51 || live.Channels[1].ID != 53 ||
		test.ChannelID != 52 || test.Environment != "test" || len(test.Channels) != 1 {
		t.Fatal("binding groups lost environments or currency aliases")
	}
	raw, err := json.Marshal(view)
	if err != nil {
		t.Fatal(err)
	}
	var projection map[string]any
	if json.Unmarshal(raw, &projection) != nil || projection["channel_id"] != nil || projection["merchant_id"] != nil {
		t.Fatal("ambiguous legacy projection was exposed")
	}
	for _, channel := range admin.document.Channels {
		if bytes.Contains(raw, []byte(channel.PrivateKeyFile)) || channel.Enabled {
			t.Fatal("private path exposed or disabled channel enabled")
		}
	}
	if bytes.Contains(raw, []byte("PRIVATE KEY")) || bytes.Contains(raw, []byte(admin.path)) {
		t.Fatal("private configuration leaked")
	}
	for _, operation := range []func(context.Context, payments.PancakeAdminRequest) (any, error){admin.Catalog, admin.Pair, admin.Save} {
		if _, err := operation(context.Background(), payments.PancakeAdminRequest{}); !errors.Is(err, payments.ErrConflict) {
			t.Fatal("ambiguous legacy operation was admitted", err)
		}
	}
	for _, id := range []int64{-1, 999} {
		if _, err := admin.Save(context.Background(), payments.PancakeAdminRequest{ChannelID: id}); !errors.Is(err, payments.ErrInvalid) {
			t.Fatal("invalid selected channel accepted", err)
		}
	}
	adapter, currencies, err := admin.probe(payments.PancakeAdminRequest{ChannelID: 53})
	if err != nil || adapter.Channel().Environment != "live" || !reflect.DeepEqual(currencies, []string{"USD", "CNY"}) {
		t.Fatal("alias selection did not bind the probe to its currency group", err)
	}
	if len(wallet.Catalog()) != 0 || quotes.next != 0 {
		t.Fatal("configuration access admitted payments")
	}
	if current, err := os.ReadFile(admin.path); err != nil || !bytes.Equal(before, current) {
		t.Fatal("read or rejected request changed configuration")
	}
}

func TestPancakeManagementSavesOnlySelectedDisabledCurrencyGroup(t *testing.T) {
	admin, wallet, _, quotes := pancakeManagementBindingsFixture(t)
	before := append([]multiPaymentChannel(nil), admin.document.Channels...)
	request := payments.PancakeAdminRequest{
		ChannelID: 53, MerchantID: before[0].Merchant,
		StoreID: before[0].StoreID, ProductID: before[0].ProductID,
		ReturnURL: "https://example.test/selected", PrivateKey: adminPrivateKey(t),
	}
	if _, err := admin.Save(context.Background(), request); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(admin.path)
	if err != nil || bytes.Contains(raw, []byte("PRIVATE KEY")) {
		t.Fatal("private key was not saved separately", err)
	}
	var saved multiPaymentDocument
	if _, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&saved); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(saved.Channels[1], before[1]) {
		t.Fatal("saving production changed the independent test binding")
	}
	for _, index := range []int{0, 2} {
		channel := saved.Channels[index]
		expected := before[index]
		expected.ReturnURL, expected.PrivateKeyFile = request.ReturnURL, channel.PrivateKeyFile
		if channel.PrivateKeyFile == before[index].PrivateKeyFile ||
			channel.PrivateKeyFile != saved.Channels[0].PrivateKeyFile ||
			!reflect.DeepEqual(channel, expected) {
			t.Fatal("selected aliases lost their common key or changed immutable admission policy")
		}
		if info, err := os.Stat(channel.PrivateKeyFile); err != nil || info.Mode().Perm() != 0600 {
			t.Fatal("replacement private key is unavailable or public", err)
		}
		if _, err := os.Stat(before[index].PrivateKeyFile); err != nil {
			t.Fatal("old private key was destroyed")
		}
	}
	if len(wallet.Catalog()) != 0 || quotes.next != 0 {
		t.Fatal("saving configuration enabled checkout or prepared money")
	}
	adapters, err := loadMultiPaymentAdapters(raw, nil)
	if err != nil || len(adapters) != 3 {
		t.Fatal("configuration is not restartable", err)
	}
	for _, adapter := range adapters {
		if adapter.Channel().Enabled {
			t.Fatal("restart enabled a disabled channel")
		}
	}
	value, err := admin.Configuration(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	bindings := value.(pancakeConfiguration).Bindings
	if len(bindings) != 2 || bindings[0].ReturnURL != request.ReturnURL || bindings[1].ReturnURL != before[1].ReturnURL {
		t.Fatal("readback did not preserve both selected and independent bindings")
	}
}

func TestPancakeManagementSeparatesDifferentReturnTargets(t *testing.T) {
	admin, _, _, _ := pancakeManagementFixture(t)
	document := admin.document
	document.Channels = append([]multiPaymentChannel(nil), document.Channels...)
	document.Channels[1].ReturnURL = "https://example.test/other-currency"
	admin, _, _, _ = pancakeManagementDocumentFixture(t, document)
	value, err := admin.Configuration(context.Background())
	if err != nil || len(value.(pancakeConfiguration).Bindings) != 2 {
		t.Fatal("different checkout return targets were silently coupled", err)
	}
	if _, err := admin.selected(0); !errors.Is(err, payments.ErrConflict) {
		t.Fatal("a missing selector could overwrite an independent return target")
	}
	if _, err := admin.Save(context.Background(), payments.PancakeAdminRequest{
		ChannelID: 53, StoreID: document.Channels[1].StoreID, ProductID: document.Channels[1].ProductID,
		ReturnURL: document.Channels[0].ReturnURL,
	}); err != nil {
		t.Fatal(err)
	}
	value, err = admin.Configuration(context.Background())
	if err != nil || len(value.(pancakeConfiguration).Bindings) != 1 || len(value.(pancakeConfiguration).Channels) != 2 {
		t.Fatal("equivalent currency bindings did not regroup after explicit save", err)
	}
}
