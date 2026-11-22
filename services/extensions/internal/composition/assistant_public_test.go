package composition

import (
	"context"
	"encoding/json"
	"errors"
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
	s := assistantPublicSnapshot{origin: "https://actual.example", checkins: p}
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
	n := assistantNavigation{authority: a, store: p, market: m}
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
