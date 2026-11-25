package webcompat

import (
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcpd"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type ssmcpdModule struct{ handler http.Handler }

func (*ssmcpdModule) Name() string            { return "ssmcpd" }
func (m *ssmcpdModule) Handler() http.Handler { return m.handler }

func NewSSMCPDModule(auth *Authenticator, s *ssmcpd.Service) modules.Module {
	mux := http.NewServeMux()
	registerSSMCPD(mux, auth, s)
	return &ssmcpdModule{moduleUserCredential(mux)}
}

func registerSSMCPD(mux RouteRegistrar, auth *Authenticator, s *ssmcpd.Service) {
	if s == nil {
		return
	}
	registerSSMCPDCommerce(mux, auth, s)
	route := func(admin bool) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			actor, err := auth.Verify(r.Context(), r)
			if err != nil {
				Respond(w, nil, err)
				return
			}
			if admin && actor.GetPlatformLevel() < 5 {
				Respond(w, nil, store.ErrForbidden)
				return
			}
			Respond(w, map[string]any{
				"service":                      s,
				"traffic_portal_available":     s.TrafficPortal != "",
				"account_usage_api_configured": s.MeteringConfigured(),
				"credit_grant_configured":      s.CommerceConfigured(),
			}, nil)
		}
	}
	mux.HandleFunc("GET /api/plugins/ssmcpd/capabilities", route(false))
	mux.HandleFunc("GET /api/plugins/ssmcpd/admin/capabilities", route(true))
	mux.HandleFunc("GET /api/plugins/ssmcpd/usage", func(w http.ResponseWriter, r *http.Request) {
		actor, err := auth.Verify(r.Context(), r)
		if err != nil {
			Respond(w, nil, err)
			return
		}
		if r.URL.RawQuery != "" {
			Respond(w, nil, store.ErrInvalid)
			return
		}
		quota, err := s.Quota(r.Context(), actor.GetUserId())
		if err != nil {
			Respond(w, nil, err)
			return
		}
		Respond(w, quota, nil)
	})
}
