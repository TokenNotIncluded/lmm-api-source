// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// RouteRegistrar is shared by Web adapters and modules with their own routes.
type RouteRegistrar = modules.RouteRegistrar

type routeRegistry struct {
	mux     *http.ServeMux
	host    *modules.Host
	plugins PluginController
	owners  []string
}

// forPlugins assigns ownership at registration. Shared routes remain available
// while any owner is enabled; routes with no owner are host administration.
func (routes routeRegistry) forPlugins(names ...string) RouteRegistrar {
	routes.owners = names
	return routes
}

func (routes routeRegistry) Handle(pattern string, handler http.Handler) {
	routes.mux.Handle(pattern, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		release, err := routes.host.TryAcquire()
		if err != nil {
			w.Header().Set("Retry-After", "1")
			Respond(w, nil, err)
			return
		}
		defer release()
		if len(routes.owners) != 0 && !store.Missing(routes.plugins) {
			enabled := false
			for _, name := range routes.owners {
				if routes.plugins.IsEnabled(name) {
					enabled = true
					break
				}
			}
			if !enabled {
				Respond(w, nil, store.ErrUnavailable)
				return
			}
		}
		handler.ServeHTTP(w, r)
	}))
}

func (routes routeRegistry) HandleFunc(pattern string, handler func(http.ResponseWriter, *http.Request)) {
	routes.Handle(pattern, http.HandlerFunc(handler))
}
