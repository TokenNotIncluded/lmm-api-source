package composition

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/url"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type assistantCheckinConfiguration interface {
	PublicConfiguration(context.Context) (checkins.PublishedConfiguration, error)
}

type assistantPublicSnapshot struct {
	origin        string
	local         bool
	checkins      assistantCheckinConfiguration
	pluginEnabled func(string) bool
}

func (s assistantPublicSnapshot) PublicSnapshot(ctx context.Context) (assistant_services.PublicConfigurationSnapshot, error) {
	value := assistant_services.PublicConfigurationSnapshot{ServiceRoot: s.origin, LocalHTTP: s.local && strings.HasPrefix(s.origin, "http://")}
	var revision int64
	if !access.Missing(s.checkins) && s.pluginEnabled != nil && s.pluginEnabled("checkins") {
		policy, err := s.checkins.PublicConfiguration(ctx)
		if err != nil {
			return value, access.ErrUnavailable
		}
		revision = policy.Revision
		value.CheckinEnabled = policy.Enabled
		value.CheckinMinCredits = policy.MinCredits
		value.CheckinMaxCredits = policy.MaxCredits
	}
	// A setup guide does not enable private credential import. This projection
	// receives no reviewed host setting for that separate action.
	raw, err := json.Marshal(struct {
		Configuration  assistant_services.PublicConfigurationSnapshot
		PolicyRevision int64
	}{value, revision})
	if err != nil {
		return value, access.ErrUnavailable
	}
	digest := sha256.Sum256(raw)
	value.Revision = hex.EncodeToString(digest[:])
	return value, nil
}

type assistantPublicProduct interface {
	PublicProduct(context.Context, string) (map[string]any, error)
}
type assistantPublicMarket interface {
	PublicWebDetail(context.Context, string, string, string) (toolmarket.WebDetail, error)
}

// Dynamic links use the same public, user-visible domain readers as the actual
// console. This adapter cannot buy, grant access, or resolve another user's ID.
type assistantNavigation struct {
	authority     access.Authority
	store         assistantPublicProduct
	market        assistantPublicMarket
	pluginEnabled func(string) bool
}

func (n assistantNavigation) Navigation(ctx context.Context, credential string, input assistant_services.NavigationInput) (json.RawMessage, error) {
	p, err := n.authority.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return nil, err
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return nil, access.ErrForbidden
	}
	var path string
	var query map[string]any
	switch input.Page {
	case "store-product":
		if !access.ID(input.Identifier) || input.Query != "" || input.Section != "" {
			return nil, access.ErrInvalid
		}
		if access.Missing(n.store) || n.pluginEnabled == nil || !n.pluginEnabled("store") {
			return nil, access.ErrUnavailable
		}
		product, e := n.store.PublicProduct(ctx, input.Identifier)
		if e != nil {
			if errors.Is(e, store.ErrNotFound) {
				return nil, access.ErrNotFound
			}
			return nil, access.ErrUnavailable
		}
		id, ok := product["id"].(string)
		if !ok || id != input.Identifier {
			return nil, access.ErrConflict
		}
		path, query = "/store/products/"+url.PathEscape(id), map[string]any{}
	case "tool-market-service":
		if !access.ID(input.Identifier) || input.Query != "" || input.Section != "" {
			return nil, access.ErrInvalid
		}
		if access.Missing(n.market) || n.pluginEnabled == nil || !n.pluginEnabled("toolmarket") {
			return nil, access.ErrUnavailable
		}
		detail, e := n.market.PublicWebDetail(ctx, credential, strconv.FormatInt(p.UserID, 10), input.Identifier)
		if e != nil {
			if errors.Is(e, toolmarket.ErrMissing) {
				return nil, access.ErrNotFound
			}
			if errors.Is(e, toolmarket.ErrDenied) || errors.Is(e, toolmarket.ErrAuth) {
				return nil, access.ErrForbidden
			}
			return nil, access.ErrUnavailable
		}
		if detail.Service.ID != input.Identifier || detail.Version.ServiceID != detail.Service.ID || detail.Version.ID != detail.Service.LiveVersionID {
			return nil, access.ErrConflict
		}
		path, query = "/tool-market", map[string]any{"service_id": detail.Service.ID}
	default:
		// Drawing and non-self user/log targets need their own typed live
		// authority projection. Metadata and the model's ID are insufficient.
		return nil, access.ErrUnavailable
	}
	return json.Marshal(map[string]any{"ok": true, "status": "navigation_ready", "path": path, "query": query})
}
