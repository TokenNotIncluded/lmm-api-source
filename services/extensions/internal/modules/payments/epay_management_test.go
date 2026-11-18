// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package payments

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

type epayConfigurationSource struct {
	value *pb.EPayConfiguration
	err   error
}

func (s *epayConfigurationSource) PaymentEPayConfiguration(context.Context) (*pb.EPayConfiguration, error) {
	return s.value, s.err
}
func epayConfigurationFixture(t *testing.T, enabled bool, admission map[int64]bool) (*EPayAdministration, *epayConfigurationSource, *EPay) {
	t.Helper()
	adapter, err := NewCoreEPay(EPayConfig{
		Channel:     Channel{ID: 55, Provider: "epay", Merchant: "123", Environment: "live", Currency: "LDC", Enabled: enabled},
		PaymentType: "epay", Endpoint: "https://credit.linux.do/epay",
		NotifyURL: "https://example.test/callbacks/55", ReturnURL: "https://example.test/local-old",
	}, epaySignerFunc(func(context.Context, int64) (Checkout, error) {
		t.Fatal("configuration read dispatched a payment")
		return Checkout{}, ErrUnavailable
	}))
	if err != nil {
		t.Fatal(err)
	}
	wallet, err := New(newMemory(), newCore(adapter.Channel()), adapter)
	if err != nil {
		t.Fatal(err)
	}
	source := &epayConfigurationSource{value: &pb.EPayConfiguration{
		Revision: strings.Repeat("a", 64),
		Bindings: []*pb.EPayConfigurationBinding{{
			ChannelId: 55, MerchantId: "123", Environment: "live", Currency: "LDC", PaymentType: "epay",
			Endpoint: "https://credit.linux.do/epay", NotifyUrl: "https://example.test/callbacks/55",
			ReturnUrl: "https://example.test/core-current", KeyConfigured: true, Enabled: false, Editable: true,
			CreditsPerMinor: "500", MaxAmountMinor: "9000000000000", MinAmountMinor: "10000",
		}},
	}}
	admin, err := NewEPayAdministration(source, wallet, admission)
	if err != nil {
		t.Fatal(err)
	}
	return admin, source, adapter
}

func TestEPayConfigurationUsesEffectiveCoreRevisionWithoutCopyingSigningMaterial(t *testing.T) {
	admin, source, adapter := epayConfigurationFixture(t, false, nil)
	value, err := admin.Configuration(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	view := value.(epayConfiguration)
	binding := view.Bindings[0]
	if binding.MinAmountMinor != "10000" || view.Revision != source.value.Revision || binding.ReturnURL != source.value.Bindings[0].ReturnUrl ||
		!binding.Editable || !binding.AdapterConfigured || !binding.AdapterMatches || binding.WalletEnabled || binding.SubscriptionEnabled {
		t.Fatal("effective policy or disabled admission was lost")
	}
	source.value.Revision, source.value.Bindings[0].ReturnUrl = strings.Repeat("b", 64), "https://example.test/core-updated"
	value, err = admin.Configuration(context.Background())
	if err != nil || value.(epayConfiguration).Revision != source.value.Revision ||
		value.(epayConfiguration).Bindings[0].ReturnURL != source.value.Bindings[0].ReturnUrl {
		t.Fatal("projection cached a stale Core configuration", err)
	}
	if adapter.cfg.ReturnURL != "https://example.test/local-old" || adapter.cfg.MerchantKey != "" || len(admin.wallet.Catalog()) != 0 {
		t.Fatal("safe read rewrote an adapter, copied a key or activated checkout")
	}
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"secret_key", "webhook_secret", "private_key_file", "local-old"} {
		if strings.Contains(string(raw), forbidden) {
			t.Fatal("private or stale configuration escaped", forbidden)
		}
	}
}

func TestEPayConfigurationCannotElevateLocalAdmissionOrMismatchedBindings(t *testing.T) {
	for _, kind := range []string{"wallet", "subscription", "merchant", "missing", "core-enabled", "history"} {
		t.Run(kind, func(t *testing.T) {
			admin, source, _ := epayConfigurationFixture(t, kind == "wallet", map[int64]bool{55: kind == "subscription"})
			switch kind {
			case "merchant":
				source.value.Bindings[0].MerchantId = "456"
			case "missing":
				source.value.Bindings[0].ChannelId = 99
			case "core-enabled":
				source.value.Bindings[0].Enabled = true
			case "history":
				source.value.Bindings[0].Editable = false
			}
			value, err := admin.Configuration(context.Background())
			if err != nil || value.(epayConfiguration).Bindings[0].Editable {
				t.Fatal("locked policy became editable", err)
			}
		})
	}
}

func TestEPayConfigurationPreservesUnavailableAndMalformedCoreResults(t *testing.T) {
	admin, source, _ := epayConfigurationFixture(t, false, nil)
	source.err = errors.New("synthetic private upstream detail")
	if _, err := admin.Configuration(context.Background()); !errors.Is(err, ErrUnavailable) || strings.Contains(err.Error(), "private") {
		t.Fatal("upstream failure was hidden or leaked", err)
	}
	source.err = nil
	source.value.Revision = "not-a-revision"
	if _, err := admin.Configuration(context.Background()); !errors.Is(err, ErrUnavailable) {
		t.Fatal("bad revision accepted", err)
	}
}

func TestEPayConfigurationMinimumComesFromCoreAndRejectsMalformedBounds(t *testing.T) {
	admin, source, adapter := epayConfigurationFixture(t, false, nil)
	adapter.cfg.Channel.MinAmountMinor = 1
	value, err := admin.Configuration(context.Background())
	if err != nil || value.(epayConfiguration).Bindings[0].MinAmountMinor != "10000" {
		t.Fatal("Core wallet floor was replaced by the local hint", err)
	}
	source.value.Bindings[0].MinAmountMinor = ""
	value, err = admin.Configuration(context.Background())
	if err != nil || value.(epayConfiguration).Bindings[0].MinAmountMinor != "1" {
		t.Fatal("older default-floor projection became incompatible", err)
	}
	for _, floor := range []string{"0", "-1", "01", "9000000000001", "999999999999999999999999", "not-a-number"} {
		source.value.Bindings[0].MinAmountMinor = floor
		if _, err := admin.Configuration(context.Background()); !errors.Is(err, ErrUnavailable) {
			t.Fatal("invalid floor projection accepted", floor)
		}
	}
	source.value.Bindings[0].MinAmountMinor = "10000"
	source.value.Bindings[0].MaxAmountMinor = "9999"
	if _, err := admin.Configuration(context.Background()); !errors.Is(err, ErrUnavailable) {
		t.Fatal("minimum above maximum accepted")
	}
}
