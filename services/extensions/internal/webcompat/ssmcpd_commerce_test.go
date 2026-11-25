package webcompat

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcpd"
	"net/http"
	"strings"
	"testing"
)

func TestSSMCPDCommerceRequiresLivePersonalSessionBeforeBody(t *testing.T) {
	s, err := ssmcpd.New(ssmcpd.Config{Mode: "relay", PublicOrigin: "https://relay.invalid"})
	if err != nil {
		t.Fatal(err)
	}
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return sessionSelf(), nil }}
	mux := http.NewServeMux()
	registerSSMCPDCommerce(mux, newAuth(t, core), s)
	id := strings.Repeat("8", 64)
	for _, tc := range []struct{ method, path, body string }{
		{"GET", "/api/plugins/ssmcpd/offer", ""}, {"GET", "/api/plugins/ssmcpd/orders/" + id, ""}, {"POST", "/api/plugins/ssmcpd/orders", "{"}, {"POST", "/api/plugins/ssmcpd/orders/" + id + "/redeem", "{"}, {"POST", "/api/plugins/ssmcpd/orders/" + id + "/refund-request", "{"},
	} {
		for _, token := range []string{"", hostToken} {
			w := pluginRequest(mux, tc.method, tc.path, tc.body, token)
			if w.Code != 401 {
				t.Fatalf("%s got %d", tc.path, w.Code)
			}
		}
	}
	for _, test := range []struct{ url, body string }{{"/api/plugins/ssmcpd/orders", `{"user_id":"2","payer":"other"}`}, {"/api/plugins/ssmcpd/orders/" + id + "/redeem", `{"bytes":"1"}`}, {"/api/plugins/ssmcpd/orders/" + id + "/refund-request", `{"user_id":"3"}`}, {"/api/plugins/ssmcpd/orders/" + id + "/redeem?user_id=9", `{}`}} {
		w := pluginRequest(mux, "POST", test.url, test.body, webToken)
		if w.Code != 422 {
			t.Fatalf("got %d: %s", w.Code, w.Body.String())
		}
	}
}
