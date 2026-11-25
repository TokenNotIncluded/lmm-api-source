package webcompat

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcpd"
	"net/http"
	"testing"
)

func TestSSMCPDSalesPolicyChecksAdministratorBeforeReadingInput(t *testing.T) {
	s, err := ssmcpd.New(ssmcpd.Config{Mode: "relay", PublicOrigin: "https://relay.invalid"})
	if err != nil {
		t.Fatal(err)
	}
	self := sessionSelf()
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return self, nil }}
	mux := http.NewServeMux()
	registerSSMCPDCommerce(mux, newAuth(t, core), s)
	const path = "/api/plugins/ssmcpd/admin/sales-policy"
	for _, method := range []string{"GET", "PUT"} {
		self.PlatformLevel = 1
		for _, tc := range []struct {
			token string
			code  int
		}{{"", 401}, {hostToken, 401}, {webToken, 403}} {
			w := pluginRequest(mux, method, path, "{", tc.token)
			if w.Code != tc.code {
				t.Fatalf("%s: %d expected %d", method, w.Code, tc.code)
			}
		}
	}
	self.PlatformLevel = 5
	w := pluginRequest(mux, "PUT", path, `{"monthly_limit_bytes":"4","unit_credits":"7","unexpected":"x"}`, webToken)
	if w.Code != 422 {
		t.Fatalf("invalid input: %d", w.Code)
	}
}
