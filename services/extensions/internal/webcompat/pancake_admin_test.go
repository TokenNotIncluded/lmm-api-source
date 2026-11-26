package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

type pancakeAdminProbe struct {
	calls     int
	channelID int64
}

func (p *pancakeAdminProbe) Configuration(context.Context) (any, error) {
	p.calls++
	return map[string]any{"contract": "extensions.pancake-configuration.v1", "private_key_configured": true}, nil
}

func (p *pancakeAdminProbe) Catalog(_ context.Context, request PancakeAdminRequest) (any, error) {
	p.calls++
	p.channelID = request.ChannelID
	return map[string]any{"stores": []any{}}, nil
}

func TestPancakeAdminMountedWithPaymentLifecycleAndLegacyEnvelope(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	core.fn = func(context.Context, string) (*pb.AuthorizeResponse, error) {
		self := sessionSelf()
		self.PlatformLevel = 6
		return self, nil
	}
	probe := &pancakeAdminProbe{}
	plugins := &pluginControllerFake{enabled: map[string]bool{"payments": true}}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{PancakeAdmin: probe, Plugins: plugins})
	request := func(action string) *httptest.ResponseRecorder {
		return pluginRequest(handler, "POST", "/api/option/waffo-pancake/"+action, `{}`, webToken)
	}
	if w := request("catalog"); w.Code != 503 || probe.calls != 0 {
		t.Fatal("admin bypassed preparing state")
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	for _, action := range []string{"catalog", "pair", "save"} {
		w := request(action)
		var body map[string]any
		if w.Code != 200 || json.Unmarshal(w.Body.Bytes(), &body) != nil || body["message"] != "success" || body["success"] != true {
			t.Fatal("legacy Web envelope or normal router missing", action, w.Code)
		}
	}
	if w := pluginRequest(handler, "GET", "/api/option/waffo-pancake/config", "", webToken); w.Code != 200 || !strings.Contains(w.Body.String(), "extensions.pancake-configuration.v1") || w.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("safe configuration readback missing", w.Code)
	}
	plugins.enabled["payments"] = false
	if w := request("catalog"); w.Code != 503 || probe.calls != 4 {
		t.Fatal("admin bypassed disabled payment module")
	}
	if w := pluginRequest(handler, "GET", "/api/option/waffo-pancake/config", "", webToken); w.Code != 503 || probe.calls != 4 {
		t.Fatal("configuration bypassed disabled payment module")
	}
	plugins.enabled["payments"] = true
	host.BeginDrain()
	if w := request("catalog"); w.Code != 503 || probe.calls != 4 {
		t.Fatal("admin bypassed drain")
	}
}

func TestPancakeConfigurationRequiresOwnerSession(t *testing.T) {
	for _, tc := range []struct {
		name, token string
		level       int32
		want        int
	}{
		{"owner", webToken, 6, 200},
		{"admin", webToken, 5, 403},
		{"anonymous", "", 6, 401},
		{"host", hostToken, 6, 401},
	} {
		t.Run(tc.name, func(t *testing.T) {
			core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
				self := sessionSelf()
				self.PlatformLevel = tc.level
				return self, nil
			}}
			probe := &pancakeAdminProbe{}
			mux := http.NewServeMux()
			RegisterPancakeAdmin(mux, newAuth(t, core), probe)
			w := pluginRequest(mux, "GET", "/api/option/waffo-pancake/config", "", tc.token)
			if w.Code != tc.want || (probe.calls == 1) != (tc.want == 200) {
				t.Fatal("configuration authorization mismatch", w.Code, probe.calls)
			}
		})
	}
}

func TestPancakeUnconfiguredReadRequiresOwnerAndKeepsWritesUnmounted(t *testing.T) {
	for _, tc := range []struct {
		name, token string
		level       int32
		status      int
	}{
		{"owner", webToken, 6, http.StatusNotFound},
		{"admin", webToken, 5, http.StatusForbidden},
		{"anonymous", "", 6, http.StatusUnauthorized},
	} {
		t.Run(tc.name, func(t *testing.T) {
			core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
				self := sessionSelf()
				self.PlatformLevel = tc.level
				return self, nil
			}}
			mux := http.NewServeMux()
			RegisterPancakeAdmin(mux, newAuth(t, core), nil)
			w := pluginRequest(mux, "GET", "/api/option/waffo-pancake/config", "", tc.token)
			var body struct {
				Code string `json:"code"`
			}
			if w.Code != tc.status || json.Unmarshal(w.Body.Bytes(), &body) != nil {
				t.Fatalf("unexpected configuration response: %d", w.Code)
			}
			unconfigured := body.Code == "PAYMENT_PANCAKE_NOT_CONFIGURED"
			if unconfigured != (tc.name == "owner") || (unconfigured && w.Header().Get("Cache-Control") != "no-store") {
				t.Fatalf("unexpected configuration disclosure or caching: %d", w.Code)
			}
			for _, action := range []string{"catalog", "pair", "save"} {
				if result := pluginRequest(mux, "POST", "/api/option/waffo-pancake/"+action, "{}", tc.token); result.Code != http.StatusNotFound {
					t.Fatalf("unconfigured action %s was mounted", action)
				}
			}
		})
	}
}

func TestPancakeAdminReportsPolicyConflictAndUncertainPersistence(t *testing.T) {
	for _, tc := range []struct {
		err    error
		status int
		code   string
	}{
		{payments.ErrPancakePolicy, 409, "PAYMENT_CORE_POLICY_REQUIRED"},
		{&payments.ConfigurationCommittedError{Cause: errors.New("synthetic_private_error")}, 503, "PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN"},
	} {
		w := httptest.NewRecorder()
		respondPancakeAdmin(w, nil, tc.err)
		if w.Code != tc.status || !strings.Contains(w.Body.String(), tc.code) || strings.Contains(w.Body.String(), "synthetic_private_error") {
			t.Fatal("lost stable configuration failure", w.Code)
		}
	}
	w := httptest.NewRecorder()
	respondPancakeAdmin(w, &payments.PancakePairResult{StoreID: "STO_partial", StoreName: "LMM API", OrphanStore: true}, errors.New("synthetic_private_error"))
	if w.Code != 502 || !strings.Contains(w.Body.String(), `"orphan_store":true`) || !strings.Contains(w.Body.String(), "STO_partial") || strings.Contains(w.Body.String(), "synthetic_private_error") {
		t.Fatal("partial provider creation lost safe recovery identity")
	}
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

func TestPancakeAdminForwardsExplicitBindingToEachAction(t *testing.T) {
	for _, action := range []string{"catalog", "pair", "save"} {
		for _, tc := range []struct {
			body      string
			status    int
			channelID int64
		}{
			{`{"channel_id":51}`, 200, 51},
			{`{"channel_id":"51"}`, 422, 0},
		} {
			t.Run(action+"/"+tc.body, func(t *testing.T) {
				core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
					self := sessionSelf()
					self.PlatformLevel = 6
					return self, nil
				}}
				probe := &pancakeAdminProbe{}
				mux := http.NewServeMux()
				RegisterPancakeAdmin(mux, newAuth(t, core), probe)
				response := pluginRequest(mux, "POST", "/api/option/waffo-pancake/"+action, tc.body, webToken)
				if response.Code != tc.status || probe.channelID != tc.channelID || (probe.calls == 1) != (tc.status == 200) {
					t.Fatal("selected binding did not survive HTTP admission", response.Code, probe.channelID, probe.calls)
				}
			})
		}
	}
}
