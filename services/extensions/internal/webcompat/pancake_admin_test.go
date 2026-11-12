package webcompat

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

type pancakeAdminProbe struct{ calls int }

func (p *pancakeAdminProbe) Catalog(context.Context, PancakeAdminRequest) (any, error) {
	p.calls++
	return map[string]any{"stores": []any{}}, nil
}
func (p *pancakeAdminProbe) Pair(ctx context.Context, r PancakeAdminRequest) (any, error) {
	return p.Catalog(ctx, r)
}
func (p *pancakeAdminProbe) Save(ctx context.Context, r PancakeAdminRequest) (any, error) {
	return p.Catalog(ctx, r)
}

func TestPancakeAdminHTTPAuthorityAndBody(t *testing.T) {
	for _, action := range []string{"catalog", "pair", "save"} {
		for _, tc := range []struct {
			name, token, body string
			level             int32
			want              int
		}{
			{"owner", webToken, `{}`, 6, 200},
			{"admin", webToken, `{}`, 5, 403},
			{"anonymous", "", `{}`, 6, 401},
			{"host", hostToken, `{}`, 6, 401},
			{"unknown field", webToken, `{"unexpected":true}`, 6, 422},
			{"trailing document", webToken, `{} {}`, 6, 422},
		} {
			t.Run(action+"/"+tc.name, func(t *testing.T) {
				core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
					self := sessionSelf()
					self.PlatformLevel = tc.level
					return self, nil
				}}
				probe := &pancakeAdminProbe{}
				mux := http.NewServeMux()
				RegisterPancakeAdmin(mux, newAuth(t, core), probe)
				r := httptest.NewRequest("POST", "/api/option/waffo-pancake/"+action, strings.NewReader(tc.body))
				if tc.token != "" {
					r.Header.Set("Authorization", "Bearer "+tc.token)
				}
				w := httptest.NewRecorder()
				mux.ServeHTTP(w, r)
				if w.Code != tc.want {
					t.Fatalf("status=%d want=%d body=%s", w.Code, tc.want, w.Body.String())
				}
				if (probe.calls == 1) != (tc.want == 200) {
					t.Fatal("provider called before admission")
				}
			})
		}
	}
}
