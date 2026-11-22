package composition

import (
	"context"
	"encoding/hex"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/planoffers"
)

// Use constructed services and the same dedicated client used by checkout.
// The existing configuration RPC authenticates that payment credential without
// preparing an intent, reserving money, or contacting a provider.
type assistantCheckoutCatalogue struct{ runtime *Runtime }

func (r assistantCheckoutCatalogue) CheckoutCatalogue(ctx context.Context) planoffers.CheckoutObservation {
	observation := planoffers.CheckoutObservation{Channels: []planoffers.CheckoutChannel{}}
	if r.runtime == nil {
		return observation
	}
	observation.Channels = payments.CheckoutCatalogue(r.runtime.Payments, r.runtime.SubscriptionPayments,
		r.runtime.StripeSubscriptionPayments, r.runtime.CreemSubscriptionPayments, r.runtime.PancakeSubscriptionPayments)
	if r.runtime.PaymentClient == nil {
		return observation
	}
	bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	configuration, err := r.runtime.PaymentClient.PaymentEPayConfiguration(bounded)
	if err == nil && configuration != nil {
		revision, parseErr := hex.DecodeString(configuration.Revision)
		observation.PaymentRPCVerified = parseErr == nil && len(revision) == 32 && hex.EncodeToString(revision) == configuration.Revision
	}
	return observation
}
