package payments

import (
	"context"
	"crypto/sha256"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

type legacyFake struct {
	Core                            // Any native prepare/price call panics.
	proof                           LegacyProof
	lookupErr, acceptErr, creditErr error
	badHash                         bool
	receipt                         LegacyReceipt
	accepts, credits, postings      int
	keys                            map[string]bool
	raw, sig                        string
}

func (f *legacyFake) LookupLegacyProof(context.Context, LegacyCallback) (LegacyProof, error) {
	if f.lookupErr != nil {
		return LegacyProof{}, f.lookupErr
	}
	if f.proof.ID == "" {
		return LegacyProof{}, ErrNotFound
	}
	return f.proof, nil
}
func (f *legacyFake) AcceptLegacyProof(_ context.Context, c LegacyCallback) (LegacyProof, error) {
	f.accepts++
	f.raw = string(c.Payload)
	f.sig = string(c.Signature)
	h := sha256.Sum256(c.Payload)
	f.proof = LegacyProof{ID: "proof_1", BasisID: "basis_1", ChannelID: c.ChannelID, OrderReference: c.OrderReference, EventID: c.EventID, OrderID: c.OrderID, PaymentID: c.PaymentID, PayloadSHA256: h[:], AcceptedUnixSeconds: 1}
	if f.badHash {
		f.proof.PayloadSHA256 = make([]byte, 32)
	}
	return f.proof, f.acceptErr
}
func (f *legacyFake) CreditLegacy(_ context.Context, id, key string) (LegacyReceipt, error) {
	f.credits++
	if f.keys == nil {
		f.keys = map[string]bool{}
	}
	if !f.keys[key] {
		f.keys[key] = true
		f.postings++
	}
	if f.creditErr != nil {
		return LegacyReceipt{}, f.creditErr
	}
	if f.receipt.OperationID != "" {
		return f.receipt, nil
	}
	return LegacyReceipt{OperationID: "apply_1", BasisID: "basis_1", ProofID: "original_proof", State: CreditApplied, JournalID: 1}, nil
}

const oldCallback = `{"eventId":"evt_old","data":{"orderMerchantExternalId":"old-order-177","orderId":"ORD_1","amount":"99.99","chargedAmount":"0.123456"}}`

func legacyService(f *legacyFake) *Service {
	return &Service{now: time.Now, core: f, adapters: map[int64]Adapter{4: &Pancake{cfg: PancakeConfig{Channel: Channel{ID: 4, Provider: "waffo_pancake"}}}}}
}
func deliverLegacy(s *Service, raw string) error {
	r := httptest.NewRequest("POST", "/", strings.NewReader(raw))
	r.Header.Set("X-Waffo-Signature", " original-signature ")
	return s.Receive(context.Background(), 4, r)
}
func TestLegacyReplayPreservesOriginalEvidence(t *testing.T) {
	f := &legacyFake{}
	s := legacyService(f)
	for i := 0; i < 2; i++ {
		if err := deliverLegacy(s, oldCallback); err != nil {
			t.Fatal(err)
		}
	}
	if f.accepts != 1 || f.postings != 1 || f.credits != 2 || !f.keys["legacy-credit:basis_1"] || f.raw != oldCallback || f.sig != " original-signature " {
		t.Fatalf("bad replay: %+v", f)
	}
}
func TestLegacyUncertainAcceptRetriesLookup(t *testing.T) {
	f := &legacyFake{acceptErr: ErrUnavailable}
	s := legacyService(f)
	if deliverLegacy(s, oldCallback) == nil || f.credits != 0 {
		t.Fatal("uncertain accept acknowledged")
	}
	if err := deliverLegacy(s, oldCallback); err != nil || f.accepts != 1 || f.postings != 1 {
		t.Fatal("proof not recovered", err)
	}
}
func TestLegacyFailuresNeverAcknowledge(t *testing.T) {
	for _, tc := range []struct {
		name string
		f    legacyFake
	}{
		{"lookup unavailable", legacyFake{lookupErr: ErrUnavailable}},
		{"lookup denied", legacyFake{lookupErr: ErrDenied}},
		{"hash", legacyFake{badHash: true}},
		{"unknown commit", legacyFake{creditErr: ErrUnavailable}},
		{"credit missing", legacyFake{creditErr: ErrNotFound}},
		{"rejected", legacyFake{receipt: LegacyReceipt{OperationID: "op", BasisID: "basis_1", ProofID: "proof_1", State: Rejected}}},
		{"wrong basis", legacyFake{receipt: LegacyReceipt{OperationID: "op", BasisID: "wrong", ProofID: "proof_1", State: CreditApplied, JournalID: 1}}},
		{"no journal", legacyFake{receipt: LegacyReceipt{OperationID: "op", BasisID: "basis_1", ProofID: "proof_1", State: CreditApplied}}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f := tc.f
			if deliverLegacy(legacyService(&f), oldCallback) == nil {
				t.Fatal("acknowledged")
			}
			if f.lookupErr != nil && f.accepts != 0 {
				t.Fatal("lookup fallback")
			}
			if f.badHash && f.credits != 0 {
				t.Fatal("hash credited")
			}
		})
	}
}
func TestLegacySelectorsNeverReinterpretNative(t *testing.T) {
	for _, raw := range []string{
		strings.Replace(oldCallback, "old-order-177", "top_17", 1), strings.Replace(oldCallback, "old-order-177", "top_017", 1),
		strings.Replace(oldCallback, `"orderMerchantExternalId":"old-order-177",`, "", 1),
		strings.Replace(oldCallback, `"eventId":"evt_old"`, `"eventId":"evt_old","eventId":"other"`, 1),
	} {
		f := &legacyFake{}
		if deliverLegacy(legacyService(f), raw) == nil || f.accepts != 0 {
			t.Fatal("bad selector", raw)
		}
	}
	f := &legacyFake{}
	s := legacyService(f)
	s.adapters[4].(*Pancake).cfg.Channel.Provider = "stripe"
	if deliverLegacy(s, oldCallback) == nil || f.accepts != 0 {
		t.Fatal("wrong provider accepted")
	}
}

func TestLegacyDurableProofSelectorsMustMatch(t *testing.T) {
	for _, mutate := range []func(*LegacyProof){
		func(p *LegacyProof) { p.ChannelID++ }, func(p *LegacyProof) { p.OrderReference = "other" },
		func(p *LegacyProof) { p.EventID = "other" }, func(p *LegacyProof) { p.OrderID = "ORD_other" },
		func(p *LegacyProof) { p.PaymentID = "PAY_other" }, func(p *LegacyProof) { p.BasisID = "" },
		func(p *LegacyProof) { p.AcceptedUnixSeconds = 0 },
	} {
		f := &legacyFake{}
		s := legacyService(f)
		if err := deliverLegacy(s, oldCallback); err != nil {
			t.Fatal(err)
		}
		mutate(&f.proof)
		before := f.credits
		if deliverLegacy(s, oldCallback) == nil || f.credits != before {
			t.Fatal("mismatched durable proof credited")
		}
	}
}
func TestLegacyBoundedHeadersAndBytes(t *testing.T) {
	for _, kind := range []string{"duplicate header", "long header", "long body"} {
		f := &legacyFake{}
		r := httptest.NewRequest("POST", "/", strings.NewReader(oldCallback))
		r.Header.Set("X-Waffo-Signature", "signature")
		switch kind {
		case "duplicate header":
			r.Header.Add("X-Waffo-Signature", "other")
		case "long header":
			r.Header.Set("X-Waffo-Signature", strings.Repeat("s", 2049))
		case "long body":
			r = httptest.NewRequest("POST", "/", strings.NewReader(strings.Repeat("x", MaxEvidence+1)))
			r.Header.Set("X-Waffo-Signature", "signature")
		}
		if legacyService(f).Receive(context.Background(), 4, r) == nil || f.accepts != 0 {
			t.Fatal(kind, "accepted")
		}
	}
}
func TestLegacyIDPrecedesEventID(t *testing.T) {
	f := &legacyFake{}
	raw := strings.Replace(oldCallback, `"eventId":"evt_old"`, `"id":"primary","eventId":"fallback"`, 1)
	if err := deliverLegacy(legacyService(f), raw); err != nil || f.proof.EventID != "primary" {
		t.Fatal("event selector changed", err)
	}
}
