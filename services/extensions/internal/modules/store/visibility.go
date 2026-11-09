package store

import "encoding/json"

func effectiveVisibility(visibility string, testMode bool) string {
	if visibility != "" {
		return visibility
	}
	if testMode {
		return "private"
	}
	return "public"
}

func productVisibility(p Product) (string, error) {
	if p.Legacy != nil {
		return effectiveVisibility(p.Legacy.Visibility, p.Legacy.TestMode), nil
	}
	if len(p.LegacyPublic) == 0 || string(p.LegacyPublic) == "null" {
		return "public", nil
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(p.LegacyPublic, &fields) != nil || fields == nil {
		return "", ErrUnavailable
	}
	var visibility string
	var testMode bool
	if raw, ok := fields["visibility"]; ok && (string(raw) == "null" || json.Unmarshal(raw, &visibility) != nil) {
		return "", ErrUnavailable
	}
	if raw, ok := fields["test_mode"]; ok && json.Unmarshal(raw, &testMode) != nil {
		return "", ErrUnavailable
	}
	return effectiveVisibility(visibility, testMode), nil
}

// viewer is a current Core-verified user, or zero for an anonymous request.
func productVisibleTo(row LocatedProduct, viewer int64) (bool, error) {
	visibility, err := productVisibility(row.Product)
	if err != nil {
		return false, err
	}
	if visibility == "private" {
		if viewer <= 0 || row.Owner != (Account{Kind: "personal", ID: viewer}) {
			return false, nil
		}
		if m := row.Product.Legacy; m != nil {
			return m.Status == "draft" || m.Status == "pending" || m.Status == "published", nil
		}
		return row.Product.Published, nil
	}
	if !row.Product.Published {
		return false, nil
	}
	if m := row.Product.Legacy; m != nil && (!m.Approved || m.Status != "published") {
		return false, nil
	}
	return visibility == "public" || visibility == "registered" && viewer > 0, nil
}

// Catalogue ownership permits a private preview. Checkout additionally requires
// an active product state and the same economic buyer as its private owner.
func productPurchaseReady(row LocatedProduct, buyer Account) (bool, error) {
	visibility, err := productVisibility(row.Product)
	if err != nil {
		return false, err
	}
	if visibility == "private" {
		if row.Owner != buyer {
			return false, nil
		}
		if m := row.Product.Legacy; m != nil {
			return m.Status == "draft" || m.Status == "pending" || m.Status == "published", nil
		}
	}
	if !row.Product.Published {
		return false, nil
	}
	if m := row.Product.Legacy; m != nil {
		return m.Approved && m.Status == "published", nil
	}
	return true, nil
}

// Older Core versions can serve normal store payments but cannot safely settle
// a same-wallet order. Require the negotiated, explicit ledger capability.
func (s *Service) selfPurchaseAvailable() bool {
	capability, ok := s.funds.(interface{ SelfPurchaseAvailable() bool })
	return ok && capability.SelfPurchaseAvailable()
}
