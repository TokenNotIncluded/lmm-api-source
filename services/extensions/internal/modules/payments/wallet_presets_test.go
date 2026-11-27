// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"errors"

	"reflect"
	"testing"
)

func TestWalletPresetCurrencyPersistence(t *testing.T) {
	path := t.TempDir()
	store := WalletPresets{Directory: path}
	initial, err := store.Read()
	if err != nil || len(initial) != 0 {
		t.Fatalf("old installation: %v %v", initial, err)
	}
	if err = store.Save("USD", []string{"500", "1000"}); err != nil {
		t.Fatal(err)
	}
	if err = store.Save("CNY", []string{"3500", "7000"}); err != nil {
		t.Fatal(err)
	}
	for _, invalid := range [][]string{nil, {"0"}, {"01"}, {"1.5"}, {"-1"}, {"9000000000001"}, {"100", "100"}} {
		if err = store.Save("CNY", invalid); !errors.Is(err, ErrInvalid) {
			t.Fatalf("accepted malformed presets: %q", invalid)
		}
	}
	if err = store.Save("../USD", []string{"100"}); !errors.Is(err, ErrInvalid) {
		t.Fatalf("accepted unknown currency: %v", err)
	}
	reopened := WalletPresets{Directory: path}
	got, err := reopened.Read()
	want := map[string][]string{"USD": {"500", "1000"}, "CNY": {"3500", "7000"}}
	if err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("persisted presets: %v %v", got, err)
	}
	if err = reopened.Save("CNY", []string{}); err != nil {
		t.Fatal(err)
	}
	want["CNY"] = []string{}
	got, err = store.Read()
	if err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("clear CNY only: %v %v", got, err)
	}
}
