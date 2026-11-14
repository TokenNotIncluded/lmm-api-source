package webcompat

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
)

type extoreTransportFake struct {
	*storeTransportFake
	token string
	actor int64
	args  []string
}

func (s *extoreTransportFake) capture(token string, actor int64, args ...string) (map[string]any, error) {
	s.token, s.actor, s.args = token, actor, args
	return map[string]any{"ok": true}, nil
}
func (s *extoreTransportFake) ExtoreAuthorize(_ context.Context, token string, actor int64, base, client string) (map[string]any, error) {
	return s.capture(token, actor, "authorize", base, client)
}
func (s *extoreTransportFake) ExtoreCatalog(_ context.Context, token string, actor int64, callback string) (any, error) {
	return s.capture(token, actor, "catalog", callback)
}
func (s *extoreTransportFake) ExtoreConnections(_ context.Context, token string, actor int64) (map[string]any, error) {
	return s.capture(token, actor, "connections")
}
func (s *extoreTransportFake) ExtoreConnectionCatalog(_ context.Context, token string, actor int64, id string) (any, error) {
	return s.capture(token, actor, "connection-catalog", id)
}
func (s *extoreTransportFake) ExtoreSyncProduct(_ context.Context, token string, actor int64, id, source string) (any, error) {
	return s.capture(token, actor, "sync", id, source)
}
func (s *extoreTransportFake) ExtoreDisconnect(_ context.Context, token string, actor int64, id string) (any, error) {
	return s.capture(token, actor, "disconnect", id)
}

// Covers HTTP admission and forwarding; not actual Core RPC or Extore business acceptance.
func TestStoreExtoreTransportGuardsAndForwarding(t *testing.T) {
	for _, route := range []struct {
		method, path, body string
		args               []string
	}{
		{"POST", "/api/store/extore/authorize", `{"base_url":"https://extore.example","client_id":"client"}`, []string{"authorize", "https://extore.example", "client"}},
		{"POST", "/api/store/extore/catalog", `{"callback_url":"https://lmm.example/callback"}`, []string{"catalog", "https://lmm.example/callback"}},
		{"GET", "/api/store/extore/connections", "", []string{"connections"}},
		{"GET", "/api/store/extore/connections/connection-1/catalog", "", []string{"connection-catalog", "connection-1"}},
		{"POST", "/api/store/extore/connections/connection-1/sync", `{"source_id":"source-1"}`, []string{"sync", "connection-1", "source-1"}},
		{"POST", "/api/store/extore/connections/connection-1/disconnect", "", []string{"disconnect", "connection-1"}},
	} {
		t.Run(route.args[0], func(t *testing.T) {
			for _, mode := range []string{"missing-credential", "core-rejected", "missing-module", "missing-port", "forwarded"} {
				t.Run(mode, func(t *testing.T) {
					core := &authCore{}
					port := &extoreTransportFake{storeTransportFake: newStoreTransportFake()}
					var service StoreService = port
					body := route.body
					expectedStatus, expectedCoreCalls := 200, 1
					switch mode {
					case "missing-credential":
						service, body, expectedStatus, expectedCoreCalls = nil, "{", 401, 0
					case "core-rejected":
						service, body, expectedStatus = nil, "{", 401
						core.fn = func(context.Context, string) (*pb.AuthorizeResponse, error) {
							return nil, status.Error(codes.Unauthenticated, "synthetic private core error")
						}
					case "missing-module":
						service, body, expectedStatus = nil, "{", 503
					case "missing-port":
						service, body, expectedStatus = newStoreTransportFake(), "{", 503
					}
					r := httptest.NewRequest(route.method, route.path, strings.NewReader(body))
					r.Header.Set("Content-Type", "application/json")
					if mode != "missing-credential" {
						r.Header.Set("Authorization", "Bearer "+webToken)
					}
					rec := httptest.NewRecorder()
					storeTransportMux(t, core, service).ServeHTTP(rec, r)
					if rec.Code != expectedStatus || core.calls != expectedCoreCalls || strings.Contains(rec.Body.String(), "synthetic private") {
						t.Fatalf("status=%d core=%d body=%s", rec.Code, core.calls, rec.Body.String())
					}
					if mode == "forwarded" {
						if port.token != webToken || port.actor != 17 || !reflect.DeepEqual(port.args, route.args) {
							t.Fatalf("actor=%d args=%v", port.actor, port.args)
						}
					} else if port.args != nil {
						t.Fatal("called business port before admission")
					}
				})
			}
		})
	}
}
