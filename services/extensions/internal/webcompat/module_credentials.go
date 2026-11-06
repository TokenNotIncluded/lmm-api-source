package webcompat

import "net/http"

func moduleUserCredential(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		r = r.Clone(r.Context())
		values := r.Header.Values("X-LMM-User-Credential")
		r.Header.Del("Authorization")
		if len(values) > 0 {
			r.Header["Authorization"] = make([]string, len(values))
			for i, v := range values {
				r.Header["Authorization"][i] = "Bearer " + v
			}
		}
		next.ServeHTTP(w, r)
	})
}
