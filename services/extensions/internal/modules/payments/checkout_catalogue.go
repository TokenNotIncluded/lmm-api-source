package payments

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"sort"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/planoffers"
)

// checkoutBinding compares only actual channel identity. Subscription products
// belong to the Core published plan price and need not equal a wallet product.
func checkoutBinding(c Channel, subscription bool, paymentType, store, product string) string {
	purpose := "topup"
	if subscription {
		purpose = "subscription"
		product = ""
	}
	if c.Provider != "epay" {
		paymentType = ""
	}
	if c.Provider != "waffo_pancake" {
		store = ""
	}
	if c.Provider != "waffo_pancake" && c.Provider != "creem" {
		product = ""
	}
	h := sha256.New()
	h.Write([]byte("assistant.checkout-binding.v1\x00"))
	for _, v := range []string{purpose, strconv.FormatInt(c.ID, 10), c.Provider, c.Merchant, c.Environment, c.Currency, paymentType, store, product} {
		var size [4]byte
		binary.BigEndian.PutUint32(size[:], uint32(len(v)))
		h.Write(size[:])
		h.Write([]byte(v))
	}
	return hex.EncodeToString(h.Sum(nil))
}

// CheckoutCatalogue reads configured objects and their current switches. It does
// not call Core, a provider, storage mutations, Preview, Prepare or Create.
func CheckoutCatalogue(wallet *Service, epay *EPaySubscriptions, stripe *StripeSubscriptions, creem *CreemSubscriptions, pancake *PancakeSubscriptions) []planoffers.CheckoutChannel {
	out := []planoffers.CheckoutChannel{}
	if wallet != nil {
		for _, c := range wallet.Catalog() {
			out = append(out, planoffers.CheckoutChannel{ChannelID: c.ID,
				Binding:          checkoutBinding(c.Channel, false, c.PaymentType, c.StoreID, c.ProductID),
				FixedAmountMinor: c.FixedAmountMinor})
		}
	}
	if epay != nil {
		for id, a := range epay.adapters {
			if epay.newPurchases[id] {
				out = append(out, planoffers.CheckoutChannel{ChannelID: id, Subscription: true,
					Binding: checkoutBinding(a.Channel(), true, a.cfg.PaymentType, "", "")})
			}
		}
	}
	if stripe != nil {
		for id, a := range stripe.adapters {
			if stripe.newPurchases[id] {
				out = append(out, planoffers.CheckoutChannel{ChannelID: id, Subscription: true,
					Binding: checkoutBinding(a.Channel(), true, "", "", "")})
			}
		}
	}
	if creem != nil {
		for id, a := range creem.adapters {
			if creem.newPurchases[id] {
				out = append(out, planoffers.CheckoutChannel{ChannelID: id, Subscription: true,
					Binding: checkoutBinding(a.Channel(), true, "", "", "")})
			}
		}
	}
	if pancake != nil {
		snapshot := pancake.pancakeSnapshot()
		for id, a := range snapshot.adapters {
			if snapshot.admission[id] {
				out = append(out, planoffers.CheckoutChannel{ChannelID: id, Subscription: true,
					Binding: checkoutBinding(a.Channel(), true, "", a.cfg.StoreID, "")})
			}
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].ChannelID != out[j].ChannelID {
			return out[i].ChannelID < out[j].ChannelID
		}
		return !out[i].Subscription && out[j].Subscription
	})
	return out
}
