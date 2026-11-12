package payments

import (
	"errors"
	"testing"
)

func TestPancakeConfigurationPublishesOnlyAfterPersistence(t *testing.T) {
	old := &Pancake{cfg: PancakeConfig{Channel: Channel{ID: 1}, ProductID: "old"}}
	next := &Pancake{cfg: old.cfg}
	next.cfg.ProductID = "new"
	wallet := &Service{adapters: map[int64]Adapter{1: old}}
	subscriptions := &PancakeSubscriptions{adapters: map[int64]*Pancake{1: old}, admission: map[int64]bool{1: true}}
	c := AttachPancakeConfiguration(wallet, subscriptions)
	prepare := func() ([]Adapter, map[int64]bool, error) { return []Adapter{next}, map[int64]bool{1: true}, nil }
	fail := errors.New("disk failed")
	if err := c.Update(prepare, func() error { return fail }); !errors.Is(err, fail) {
		t.Fatal(err)
	}
	if wallet.adapterSnapshot()[1] != old || subscriptions.pancakeSnapshot().adapters[1] != old {
		t.Fatal("published failed write")
	}
	if err := c.Update(prepare, func() error {
		if wallet.adapterSnapshot()[1] != old {
			t.Fatal("published before persistence")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if wallet.adapterSnapshot()[1] != next || subscriptions.pancakeSnapshot().adapters[1] != next {
		t.Fatal("services differ")
	}
}
