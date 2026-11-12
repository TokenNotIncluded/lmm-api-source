package payments

// forOrder keeps historical orders readable but refuses provider IO when their
// original product was not captured. A new configuration cannot supply proof
// of what an earlier checkout used.
func (p *Pancake) forOrder(o Order) (*Pancake, error) {
	b := o.PancakeBinding
	if b == nil || !validID(b.StoreID) || !validID(b.ProductID) {
		return nil, ErrUnavailable
	}
	if !providerOrderMatches(o, p.cfg.Channel) {
		return nil, ErrConflict
	}
	bound := *p
	bound.cfg.StoreID, bound.cfg.ProductID = b.StoreID, b.ProductID
	bound.cfg.ReturnURL, bound.cfg.TaxCategory = b.ReturnURL, b.TaxCategory
	bound.cfg.CheckoutRegion, bound.cfg.CheckoutLanguage = b.CheckoutRegion, b.CheckoutLanguage
	return &bound, nil
}
