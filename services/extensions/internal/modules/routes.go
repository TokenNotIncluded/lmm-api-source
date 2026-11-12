// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package modules

import "net/http"

// RouteRegistrar lets modules register routes with a standalone ServeMux or a
// host registrar that attaches admission and plugin lifecycle checks.
type RouteRegistrar interface {
	Handle(string, http.Handler)
	HandleFunc(string, func(http.ResponseWriter, *http.Request))
}
