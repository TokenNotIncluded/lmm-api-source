package webcompat

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

type paymentPortalProbe struct {
	users []int64
	err   error
}

func (p *paymentPortalProbe) PancakePortal(_ context.Context, userID int64) (*payments.CustomerPortalLink, error) {
	p.users = append(p.users, userID)
	return &payments.CustomerPortalLink{PortalURL: "https://pancake.waffo.ai/consumer/portal/store/STO_fixture#token=synthetic", ExpiresAt: "2026-11-12T12:00:00Z"}, p.err
}

func TestPaymentPortalMountKeepsSessionAndHostAuthoritySeparate(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	probe := &paymentPortalProbe{}
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{PaymentPortal: probe})
	for _, tc := range []struct {
		name, token, body, query string
		status                   int
	}{
		{"session", webToken, "", "", 200},
		{"host", hostToken, "", "", 401},
		{"anonymous", "", "", "", 401},
		{"customer body", webToken, `{"customer":"another"}`, "", 400},
		{"store query", webToken, "", "?store_id=another", 400},
	} {
		t.Run(tc.name, func(t *testing.T) {
			before := len(probe.users)
			r := httptest.NewRequest("POST", "/api/user/payments/pancake/portal"+tc.query, strings.NewReader(tc.body))
			if tc.token != "" {
				r.Header.Set("Authorization", "Bearer "+tc.token)
			}
			w := httptest.NewRecorder()
			handler.ServeHTTP(w, r)
			if w.Code != tc.status || (len(probe.users) == before+1) != (tc.status == 200) {
				t.Fatalf("status=%d provider calls=%d", w.Code, len(probe.users)-before)
			}
			if tc.status == 200 && (probe.users[before] != sessionSelf().GetUserId() || w.Header().Get("Cache-Control") != "no-store, private" || w.Header().Get("Referrer-Policy") != "no-referrer") {
				t.Fatal("portal user or private response headers lost")
			}
		})
	}
	core.fn = func(context.Context, string) (*pb.AuthorizeResponse, error) {
		self := sessionSelf()
		self.CredentialKind = pb.CredentialKind_CREDENTIAL_KIND_API_KEY
		return self, nil
	}
	r := httptest.NewRequest("POST", "/api/user/payments/pancake/portal", nil)
	r.Header.Set("Authorization", "Bearer "+webToken)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	if w.Code != http.StatusForbidden || len(probe.users) != 1 {
		t.Fatal("API key upgraded to portal session")
	}
	core.fn = nil
	probe.err = errors.New("synthetic_provider_secret")
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	if w.Code != 503 || strings.Contains(w.Body.String(), "synthetic_provider_secret") || strings.Contains(w.Body.String(), "#token") {
		t.Fatal("provider failure leaked portal material")
	}
	host.BeginDrain()
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	if w.Code != 503 || len(probe.users) != 2 {
		t.Fatal("portal bypassed drain")
	}
}
