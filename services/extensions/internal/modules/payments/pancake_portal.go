package payments

import (
	"context"
	"strconv"
	"strings"
	"time"

	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

// CustomerPortalLink contains a short-lived bearer URL. It must never enter
// payment storage, provider diagnostics or a shared response cache.
type CustomerPortalLink struct {
	PortalURL string `json:"portal_url"`
	ExpiresAt string `json:"expires_at"`
}

// PancakePortal receives only the user identity already verified by Core.
// Currency aliases may share a portal, but distinct stores/environments cannot
// be selected implicitly or by a caller-supplied customer identifier.
func (s *Service) PancakePortal(ctx context.Context, userID int64) (*CustomerPortalLink, error) {
	if userID <= 0 {
		return nil, ErrDenied
	}
	var selected *Pancake
	for _, adapter := range s.adapterSnapshot() {
		p, ok := adapter.(*Pancake)
		if !ok || !p.cfg.Channel.Enabled {
			continue
		}
		if selected != nil && (selected.cfg.Channel.namespace() != p.cfg.Channel.namespace() || selected.cfg.StoreID != p.cfg.StoreID) {
			return nil, ErrConflict
		}
		if selected == nil || p.cfg.Channel.ID < selected.cfg.Channel.ID {
			selected = p
		}
	}
	if selected == nil {
		return nil, ErrUnavailable
	}
	link, err := selected.client.Auth.CreateCustomerPortalLink(ctx, pancake.CreateCustomerPortalLinkParams{
		StoreID: selected.cfg.StoreID, BuyerIdentity: "lmm-user-" + strconv.FormatInt(userID, 10),
	})
	if err != nil || link == nil {
		return nil, ErrUnavailable
	}
	u, err := secureURL(link.PortalURL)
	expires, expiryErr := time.Parse(time.RFC3339, link.ExpiresAt)
	if err != nil || u.Hostname() != "pancake.waffo.ai" || u.Path != "/consumer/portal/store/"+selected.cfg.StoreID ||
		u.RawQuery != "" || !strings.HasPrefix(u.Fragment, "token=") || len(u.Fragment) <= len("token=") || expiryErr != nil || !expires.After(time.Now()) {
		return nil, ErrConflict
	}
	return &CustomerPortalLink{PortalURL: link.PortalURL, ExpiresAt: link.ExpiresAt}, nil
}
