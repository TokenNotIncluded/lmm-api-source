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
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/wallet"
)

type builtinSessionCore struct {
	kind pb.CredentialKind
	seen string
}

func (c *builtinSessionCore) Authorize(_ context.Context, raw string) (*pb.AuthorizeResponse, error) {
	c.seen = raw
	return &pb.AuthorizeResponse{UserId: 7, PlatformLevel: 1, CredentialKind: c.kind, Owner: &pb.Account{Kind: pb.AccountKind_ACCOUNT_KIND_PERSONAL, Id: 7}}, nil
}

type builtinWalletAuthority struct {
	wallet.Authority
	seen  string
	calls int
}

func (a *builtinWalletAuthority) Balance(_ context.Context, raw string) (wallet.Balance, error) {
	a.calls++
	a.seen = raw
	return wallet.Balance{OwnerUserID: 7, AccountID: 91, CreditsPerUSD: 500000, AvailableCredits: 33}, nil
}

type builtinWalletRecords struct{ wallet.Records }
type builtinBountyRecords struct{ bounties.Repository }
type builtinBountyFunds struct{ bounties.Funds }

func TestBuiltinHTTPWalletWorksWhileBountyExecutionIsDeferred(t *testing.T) {
	core := &builtinSessionCore{kind: pb.CredentialKind_CREDENTIAL_KIND_SESSION}
	auth := newAuth(t, core)
	authority := &builtinWalletAuthority{}
	service, err := wallet.New(authority, builtinWalletRecords{}, "https://core.example.test")
	if err != nil {
		t.Fatal(err)
	}
	handlers, err := NewBuiltinHTTP(auth, service, nil, "https://console.example.test", false)
	if err != nil {
		t.Fatal(err)
	}
	mux := http.NewServeMux()
	RegisterBuiltinHTTP(mux, handlers)
	r := httptest.NewRequest(http.MethodGet, "https://console.example.test/api/wallet/balance", nil)
	r.Header.Set("Authorization", "Bearer "+webToken)
	w := httptest.NewRecorder()
	mux.ServeHTTP(w, r)
	if w.Code != http.StatusOK || core.seen != webToken || authority.seen != webToken || authority.calls != 1 {
		t.Fatalf("wallet unavailable without bounties: status=%d calls=%d", w.Code, authority.calls)
	}
	for _, path := range []string{"/api/bounties/projects", "/api/open-source-bounties/projects", "/api/bounties/projects/old/claims/old/pay"} {
		r := httptest.NewRequest(http.MethodPost, path, strings.NewReader(`{"reward_quota":100}`))
		r.Header.Set("Authorization", "Bearer "+webToken)
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, r)
		var body struct {
			Success bool   `json:"success"`
			Code    string `json:"code"`
			Data    any    `json:"data"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
			t.Fatal(err)
		}
		if w.Code != http.StatusServiceUnavailable || body.Success || body.Code != "BOUNTY_ERROR" || body.Data != nil || authority.calls != 1 || w.Header().Get("Cache-Control") != "no-store" {
			t.Fatalf("deferred bounty accepted %s: status=%d body=%s", path, w.Code, w.Body.String())
		}
	}
}

func TestBuiltinHTTPStillRequiresSessionAuthenticator(t *testing.T) {
	_, err := NewBuiltinHTTP(nil, nil, nil, "https://console.example.test", false)
	if !errors.Is(err, store.ErrUnavailable) {
		t.Fatalf("missing session authenticator: %v", err)
	}
}

func TestBuiltinHTTPPreservesVerifiedSessionAndRejectsOtherCredentials(t *testing.T) {
	core := &builtinSessionCore{kind: pb.CredentialKind_CREDENTIAL_KIND_SESSION}
	host := []byte(strings.Repeat("h", 32))
	auth, e := NewAuthenticator(core, host)
	if e != nil {
		t.Fatal(e)
	}
	authority := &builtinWalletAuthority{}
	service, e := wallet.New(authority, builtinWalletRecords{}, "https://core.example.test")
	if e != nil {
		t.Fatal(e)
	}
	handlers, e := NewBuiltinHTTP(auth, service, &bounties.Service{Store: builtinBountyRecords{}, Funds: builtinBountyFunds{}}, "https://console.example.test", false)
	if e != nil {
		t.Fatal(e)
	}
	mux := http.NewServeMux()
	RegisterBuiltinHTTP(mux, handlers)
	session := "lmms_" + strings.Repeat("s", 40)
	request := httptest.NewRequest("GET", "https://console.example.test/api/wallet/balance", nil)
	request.Header.Set("Authorization", "Bearer "+session)
	request.Header.Set("X-User-ID", "999")
	response := httptest.NewRecorder()
	mux.ServeHTTP(response, request)
	if response.Code != 200 || core.seen != session || authority.seen != session || authority.calls != 1 {
		t.Fatalf("session was replaced: %d %s", response.Code, response.Body.String())
	}
	for _, kind := range []string{"host", "duplicate", "api-key"} {
		request = httptest.NewRequest("GET", "https://console.example.test/api/wallet/balance", nil)
		request.Header.Set("Authorization", "Bearer "+session)
		core.kind = pb.CredentialKind_CREDENTIAL_KIND_SESSION
		switch kind {
		case "host":
			request.Header.Set("Authorization", "Bearer "+string(host))
		case "duplicate":
			request.Header.Add("Authorization", "Bearer "+session)
		case "api-key":
			core.kind = pb.CredentialKind_CREDENTIAL_KIND_API_KEY
		}
		response = httptest.NewRecorder()
		mux.ServeHTTP(response, request)
		if response.Code < 400 || authority.calls != 1 {
			t.Fatalf("%s credential reached domain: %d", kind, response.Code)
		}
	}
}
func TestBuiltinHTTPPathsParticipateInHostAdmission(t *testing.T) {
	host, core := mountTransportHost(t, nil)
	manager := &pluginControllerFake{enabled: map[string]bool{"toolmarket": true}}
	calls := 0
	endpoint := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls++
		w.WriteHeader(http.StatusNoContent)
	})
	handler := Mount(host, newAuth(t, core), nil, nil, AdditionalServices{
		Plugins:  manager,
		Builtins: BuiltinHTTP{Wallet: endpoint, Bounties: endpoint},
	})
	paths := []string{"/api/wallet-transfer", "/api/wallet-transfer/42/cancel", "/api/wallet/balance", "/api/wallet/topup-link", "/api/bounties", "/api/bounties/projects/id", "/api/open-source-bounties", "/api/open-source-bounties/projects/id"}
	for _, route := range paths {
		if response := mountTransportCall(handler, http.MethodGet, route, nil); response.Code != http.StatusServiceUnavailable || response.Header().Get("Retry-After") != "1" {
			t.Fatalf("preparing %s: status=%d headers=%v", route, response.Code, response.Header())
		}
	}
	if calls != 0 {
		t.Fatal("builtin entered while host was preparing")
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	for _, route := range paths {
		if response := mountTransportCall(handler, http.MethodGet, route, nil); response.Code != http.StatusNoContent {
			t.Fatalf("ready %s: status=%d", route, response.Code)
		}
	}
	for _, route := range []string{"/api/wallet-transfer-other", "/api/wallet/balance-other", "/api/bounties-other", "/api/open-source-bounties-other"} {
		if response := mountTransportCall(handler, http.MethodGet, route, nil); response.Code != http.StatusNotFound {
			t.Fatalf("adjacent route %s: status=%d", route, response.Code)
		}
	}
	manager.enabled["toolmarket"] = false
	for _, route := range paths {
		if response := mountTransportCall(handler, http.MethodGet, route, nil); response.Code != http.StatusServiceUnavailable {
			t.Fatalf("disabled %s: status=%d", route, response.Code)
		}
	}
	if calls != len(paths) {
		t.Fatalf("disabled or adjacent route reached builtin: calls=%d", calls)
	}
}

func TestImageOnlyHTTPKeepsWalletBountyUnavailable(t *testing.T) {
	_, core := mountTransportHost(t, nil)
	handlers, err := NewBuiltinHTTP(newAuth(t, core), nil, nil, "https://console.example.test", false)
	if err != nil {
		t.Fatal(err)
	}
	for _, handler := range []http.Handler{handlers.Wallet, handlers.Bounties} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest("GET", "/", nil))
		if response.Code != http.StatusServiceUnavailable {
			t.Fatal("missing authority accepted", response.Code)
		}
	}
}
