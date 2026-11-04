package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"io"
	"net/http"
	"strings"
)

// LegacyCore is optional: native adapters and their Core implementations keep
// the existing lifecycle. Only Core chooses the frozen account and amount.
type LegacyCore interface {
	LookupLegacyProof(context.Context, LegacyCallback) (LegacyProof, error)
	AcceptLegacyProof(context.Context, LegacyCallback) (LegacyProof, error)
	CreditLegacy(context.Context, string, string) (LegacyReceipt, error)
}
type LegacyCallback struct {
	ChannelID                                   int64
	OrderReference, EventID, OrderID, PaymentID string
	Payload, Signature                          []byte
}
type LegacyProof struct {
	ID, BasisID                                 string
	ChannelID                                   int64
	OrderReference, EventID, OrderID, PaymentID string
	PayloadSHA256                               []byte
	AcceptedUnixSeconds                         int64
}
type LegacyReceipt struct {
	OperationID, BasisID, ProofID, State, RejectionCode string
	JournalID                                           int64
}

func (s *Service) receiveLegacyPancake(ctx context.Context, a Adapter, req *http.Request) (bool, error) {
	raw, sig, err := callbackBytes(req, "X-Waffo-Signature")
	if err != nil {
		return true, err
	}
	var v struct {
		ID      string `json:"id"`
		EventID string `json:"eventId"`
		Data    struct {
			Reference string `json:"orderMerchantExternalId"`
			OrderID   string `json:"orderId"`
			PaymentID string `json:"paymentId"`
		} `json:"data"`
	}
	if jsonObject(raw, &v) != nil {
		return true, ErrInvalid
	}
	// Reserved native references must never fall back after native validation
	// fails, including malformed top_ references.
	if strings.HasPrefix(v.Data.Reference, "top_") || validTopupOrderID(v.Data.Reference) {
		req.Body = io.NopCloser(bytes.NewReader(raw))
		return false, nil
	}
	if a.Channel().Provider != "waffo_pancake" || !validID(v.Data.Reference) {
		return true, ErrInvalid
	}
	event := v.ID
	if event == "" {
		event = v.EventID
	}
	if !validID(event) || !validID(v.Data.OrderID) || v.Data.PaymentID != "" && !validID(v.Data.PaymentID) {
		return true, ErrInvalid
	}
	core, ok := s.core.(LegacyCore)
	if !ok {
		return true, ErrUnavailable
	}
	c := LegacyCallback{ChannelID: a.Channel().ID, OrderReference: v.Data.Reference, EventID: event, OrderID: v.Data.OrderID, PaymentID: v.Data.PaymentID, Payload: raw, Signature: sig}
	proof, err := core.LookupLegacyProof(ctx, c)
	if errors.Is(err, ErrNotFound) {
		proof, err = core.AcceptLegacyProof(ctx, c)
	}
	if err != nil {
		return true, err
	}
	h := sha256.Sum256(raw)
	if !validID(proof.ID) || !validID(proof.BasisID) || proof.ChannelID != c.ChannelID || proof.OrderReference != c.OrderReference || proof.EventID != event || proof.OrderID != c.OrderID || proof.PaymentID != c.PaymentID || proof.AcceptedUnixSeconds <= 0 || !bytes.Equal(proof.PayloadSHA256, h[:]) {
		return true, ErrConflict
	}
	key := "legacy-credit:" + proof.BasisID
	if len(key) > 128 {
		return true, ErrConflict
	}
	r, err := core.CreditLegacy(ctx, proof.ID, key)
	if err != nil {
		return true, err
	}
	// A basis already posted by another delivery may name its original proof.
	if !validID(r.OperationID) || !validID(r.ProofID) || r.BasisID != proof.BasisID || r.State != CreditApplied || r.JournalID <= 0 || r.RejectionCode != "" {
		return true, ErrEvidence
	}
	return true, nil
}
