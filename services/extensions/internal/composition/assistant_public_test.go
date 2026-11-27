package composition

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"net/http"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type publicPolicyStub struct {
	value checkins.PublishedConfiguration
	err   error
}

func (s *publicPolicyStub) PublicConfiguration(context.Context) (checkins.PublishedConfiguration, error) {
	return s.value, s.err
}

func TestAssistantPublicSnapshotBindsActualOriginAndPublishedPolicy(t *testing.T) {
	p := &publicPolicyStub{value: checkins.PublishedConfiguration{Revision: 4, Enabled: true, MinCredits: 3, MaxCredits: 7}}
	s := assistantPublicSnapshot{origin: "https://actual.example", checkins: p, pluginEnabled: func(string) bool { return true }}
	first, err := s.PublicSnapshot(context.Background())
	if err != nil || first.ServiceRoot != s.origin || !first.CheckinEnabled || first.CheckinMinCredits != 3 || first.CheckinMaxCredits != 7 || first.CCSwitchImportEnabled || len(first.Revision) != 64 {
		t.Fatal(first, err)
	}
	s.local = true
	https, err := s.PublicSnapshot(context.Background())
	if err != nil || https.LocalHTTP {
		t.Fatal("internal local HTTP permission changed the public HTTPS origin")
	}
	p.value.Revision++
	second, err := s.PublicSnapshot(context.Background())
	if err != nil || first.Revision == second.Revision {
		t.Fatal("published revision not bound")
	}
	p.err = errors.New("private policy database failed")
	if _, err = s.PublicSnapshot(context.Background()); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("policy failure became disabled/success", err)
	}
	s.checkins = nil
	none, err := s.PublicSnapshot(context.Background())
	if err != nil || none.CheckinEnabled || none.CheckinMaxCredits != 0 || none.Revision == first.Revision {
		t.Fatal("missing module advertised a reward")
	}
}

type navigationAuthorityStub struct {
	calls  int
	denied bool
}

func (s *navigationAuthorityStub) Check(_ context.Context, credential string, account access.Account, permission access.Permission) (access.Principal, error) {
	s.calls++
	if s.denied || credential != "synthetic-original-session" || account != (access.Account{}) || permission != access.Read {
		return access.Principal{}, access.ErrForbidden
	}
	return access.Principal{UserID: 37, Account: access.Account{Kind: "personal", ID: 37}}, nil
}

type navigationProductStub struct {
	id    string
	err   error
	calls int
}

func (s *navigationProductStub) PublicProduct(_ context.Context, id string) (map[string]any, error) {
	s.calls++
	if id != "published-product" {
		return nil, store.ErrNotFound
	}
	return map[string]any{"id": s.id}, s.err
}

type navigationMarketStub struct {
	detail toolmarket.WebDetail
	calls  int
}

func (s *navigationMarketStub) PublicWebDetail(_ context.Context, credential, account, id string) (toolmarket.WebDetail, error) {
	s.calls++
	if credential != "synthetic-original-session" || account != "37" || id != "published-service" {
		return toolmarket.WebDetail{}, toolmarket.ErrDenied
	}
	return s.detail, nil
}

func TestAssistantNavigationUsesExactVisibleDomainResource(t *testing.T) {
	a := &navigationAuthorityStub{}
	p := &navigationProductStub{id: "published-product"}
	m := &navigationMarketStub{}
	m.detail.Service.ID, m.detail.Service.LiveVersionID = "published-service", "actual-version"
	m.detail.Version.ServiceID, m.detail.Version.ID = "published-service", "actual-version"
	n := assistantNavigation{authority: a, store: p, market: m, pluginEnabled: func(string) bool { return true }}
	raw, err := n.Navigation(context.Background(), "synthetic-original-session", assistant_services.NavigationInput{Page: "store-product", Identifier: "published-product"})
	var view struct {
		Path  string         `json:"path"`
		Query map[string]any `json:"query"`
	}
	if err != nil || json.Unmarshal(raw, &view) != nil || view.Path != "/store/products/published-product" || len(view.Query) != 0 {
		t.Fatal(string(raw), err)
	}
	if _, err = n.Navigation(context.Background(), "synthetic-original-session", assistant_services.NavigationInput{Page: "store-product", Identifier: "private-product"}); !errors.Is(err, access.ErrNotFound) {
		t.Fatal("private product link escaped visibility", err)
	}
	p.id = "another-product"
	if _, err = n.Navigation(context.Background(), "synthetic-original-session", assistant_services.NavigationInput{Page: "store-product", Identifier: "published-product"}); !errors.Is(err, access.ErrConflict) {
		t.Fatal("mismatched result ID accepted", err)
	}
	raw, err = n.Navigation(context.Background(), "synthetic-original-session", assistant_services.NavigationInput{Page: "tool-market-service", Identifier: "published-service"})
	if err != nil || json.Unmarshal(raw, &view) != nil || view.Path != "/tool-market" || view.Query["service_id"] != "published-service" {
		t.Fatal(string(raw), err)
	}
	m.detail.Version.ID = "stale-version"
	if _, err = n.Navigation(context.Background(), "synthetic-original-session", assistant_services.NavigationInput{Page: "tool-market-service", Identifier: "published-service"}); !errors.Is(err, access.ErrConflict) {
		t.Fatal("stale market version accepted", err)
	}
	for _, input := range []assistant_services.NavigationInput{{Page: "users", Identifier: "another-user"}, {Page: "usage-logs", Identifier: "another-user"}, {Page: "drawing"}} {
		if _, err = n.Navigation(context.Background(), "synthetic-original-session", input); !errors.Is(err, access.ErrUnavailable) {
			t.Fatal("missing typed authority advertised", input, err)
		}
	}
	before := p.calls + m.calls
	a.denied = true
	if _, err = n.Navigation(context.Background(), "synthetic-original-session", assistant_services.NavigationInput{Page: "store-product", Identifier: "published-product"}); !errors.Is(err, access.ErrForbidden) || p.calls+m.calls != before {
		t.Fatal("revoked session read domain data")
	}
}

type guideLifecycleModule string

func (m guideLifecycleModule) Name() string        { return string(m) }
func (guideLifecycleModule) Handler() http.Handler { return http.NotFoundHandler() }

type guideLifecycleCatalog struct{}

func (guideLifecycleCatalog) AvailableModels(_ context.Context, token string) (assistant_services.ModelCatalogSnapshot, error) {
	if token != "synthetic-original-session" {
		return assistant_services.ModelCatalogSnapshot{}, access.ErrForbidden
	}
	return assistant_services.ModelCatalogSnapshot{ActorUserID: 37, CheckinUser: &assistant_services.PublicCheckinUser{CheckedInToday: true}}, nil
}

func TestAssistantPublicConsumersFollowLivePluginLifecycle(t *testing.T) {
	ctx := context.Background()
	const token = "synthetic-original-session"
	installed := []modules.Module{guideLifecycleModule("checkins"), guideLifecycleModule("store"), guideLifecycleModule("toolmarket")}
	host, err := modules.NewHost(installed, []byte(strings.Repeat("h", 32)))
	if err != nil {
		t.Fatal(err)
	}
	manager, err := modules.NewPluginManager(host, installed, "")
	if err != nil {
		t.Fatal(err)
	}
	auth := &navigationAuthorityStub{}
	policy := &publicPolicyStub{value: checkins.PublishedConfiguration{Revision: 4, Enabled: true, MinCredits: 3, MaxCredits: 7}}
	product := &navigationProductStub{id: "published-product"}
	market := &navigationMarketStub{}
	market.detail.Service.ID, market.detail.Service.LiveVersionID = "published-service", "actual-version"
	market.detail.Version.ServiceID, market.detail.Version.ID = "published-service", "actual-version"
	nav := assistantNavigation{authority: auth, store: product, market: market, pluginEnabled: manager.IsEnabled}
	guides, err := assistant_services.NewPublicGuides(auth, assistantPublicSnapshot{origin: "https://actual.example", checkins: policy, pluginEnabled: manager.IsEnabled}, guideLifecycleCatalog{})
	if err != nil {
		t.Fatal(err)
	}
	facts := func(enabled bool, min, max float64) string {
		t.Helper()
		raw, e := guides.ServiceFacts(ctx, token)
		var result struct {
			Revision   string `json:"configuration_revision"`
			Activities struct {
				Checkin map[string]any `json:"daily_checkin"`
			} `json:"activities"`
		}
		if e != nil || json.Unmarshal(raw, &result) != nil || result.Revision == "" {
			t.Fatalf("facts %s: %v", raw, e)
		}
		checkin := result.Activities.Checkin
		if checkin["enabled"] != enabled {
			t.Fatalf("unexpected check-in %v", checkin)
		}
		if enabled {
			if checkin["min_quota"] != min || checkin["max_quota"] != max || checkin["checked_in_today"] != true {
				t.Fatalf("reward %v", checkin)
			}
		} else {
			for _, k := range []string{"min_quota", "max_quota", "base_min_quota", "base_max_quota", "reward_multiplier", "checked_in_today"} {
				if _, ok := checkin[k]; ok {
					t.Fatalf("disabled check-in exposed %s: %v", k, checkin)
				}
			}
		}
		return result.Revision
	}
	toggle := func(name string, on bool) {
		t.Helper()
		var e error
		if on {
			_, e = manager.Enable(name)
		} else {
			_, e = manager.Disable(name)
		}
		if e != nil || manager.IsEnabled(name) != on {
			t.Fatalf("toggle %s %v: %v", name, on, e)
		}
	}
	navigate := func(name string, on bool) {
		t.Helper()
		input := assistant_services.NavigationInput{Page: "store-product", Identifier: "published-product"}
		calls := &product.calls
		path := "/store/products/published-product"
		if name == "toolmarket" {
			input = assistant_services.NavigationInput{Page: "tool-market-service", Identifier: "published-service"}
			calls = &market.calls
			path = "/tool-market"
		}
		before := *calls
		raw, e := nav.Navigation(ctx, token, input)
		if !on {
			if !errors.Is(e, access.ErrUnavailable) || len(raw) != 0 || *calls != before {
				t.Fatalf("disabled %s called or linked: %s %v", name, raw, e)
			}
			return
		}
		var result struct {
			Path  string
			Query map[string]string
		}
		if e != nil || json.Unmarshal(raw, &result) != nil || result.Path != path || *calls != before+1 || (name == "toolmarket" && result.Query["service_id"] != input.Identifier) {
			t.Fatalf("bad enabled %s link %s: %v", name, raw, e)
		}
	}
	before := facts(true, 3, 7)
	toggle("checkins", false)
	policy.err = errors.New("disabled reward policy must not be read")
	off := facts(false, 0, 0)
	if before == off {
		t.Fatal("disabled revision stale")
	}
	navigate("store", true)
	navigate("toolmarket", true)
	policy.err = nil
	policy.value.Revision++
	policy.value.MinCredits, policy.value.MaxCredits = 8, 12
	toggle("checkins", true)
	if facts(true, 8, 12) == off {
		t.Fatal("re-enabled revision stale")
	}
	for _, pair := range [][2]string{{"store", "toolmarket"}, {"toolmarket", "store"}} {
		toggle(pair[0], false)
		navigate(pair[0], false)
		navigate(pair[1], true)
		facts(true, 8, 12)
		toggle(pair[0], true)
		navigate(pair[0], true)
	}
}
