package webcompat

import (
	"context"
	"net/http"
	"net/http/httptest"
	"reflect"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func TestModuleUserCredentials(t *testing.T) {
	for _, module := range []struct {
		name, path string
		handler    func(*Authenticator) http.Handler
	}{
		{"community", "/api/user/self/profile-share", func(auth *Authenticator) http.Handler { return NewCommunityModule(auth, nil).Handler() }},
		{"acquisition", "/api/acquisition/self-report", func(auth *Authenticator) http.Handler { return NewAcquisitionModule(auth, nil).Handler() }},
		{"operations", "/api/release-notes/latest", func(auth *Authenticator) http.Handler { return NewOperationsModule(auth, nil).Handler() }},
	} {
		for _, tc := range []struct {
			name        string
			credentials []string
			code, calls int
		}{
			{"missing", nil, http.StatusUnauthorized, 0},
			{"session", []string{webToken}, http.StatusForbidden, 1},
			{"duplicate", []string{webToken, webToken}, http.StatusUnauthorized, 0},
		} {
			t.Run(module.name+"/"+tc.name, func(t *testing.T) {
				core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
					return nil, status.Error(codes.PermissionDenied, "Core denied the current session")
				}}
				h := module.handler(newAuth(t, core))
				r := httptest.NewRequest(http.MethodGet, module.path, nil)
				r.Header.Set("Authorization", "Bearer "+webToken)
				for _, value := range tc.credentials {
					r.Header.Add("X-LMM-User-Credential", value)
				}
				original := r.Header.Clone()
				w := httptest.NewRecorder()
				h.ServeHTTP(w, r)
				if w.Code != tc.code || core.calls != tc.calls {
					t.Fatalf("status=%d calls=%d body=%s", w.Code, core.calls, w.Body.String())
				}
				if core.calls == 1 && core.tokens[0] != webToken {
					t.Fatal("original user credential changed", core.tokens)
				}
				if !reflect.DeepEqual(r.Header, original) {
					t.Fatal("module mutated the caller's headers")
				}
			})
		}
	}
}
