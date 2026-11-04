// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"testing"
	"time"
)

type rankingAdmin struct{}

func (rankingAdmin) Verify(context.Context, *http.Request) (*pb.AuthorizeResponse, error) {
	return &pb.AuthorizeResponse{UserId: 1, PlatformLevel: 5}, nil
}

func TestCoreCommunityRuntimeLive(t *testing.T) {
	socket := os.Getenv("LMM_TEST_CORE_RANK_SOCKET")
	path := os.Getenv("LMM_RANK_RUNTIME_FIXTURE")
	if socket == "" || path == "" {
		t.Skip("requires actual Core and formally installed isolated runtime databases")
	}
	var fixture map[string]struct{ Database, RuntimeURLFile string }
	raw, err := coreclient.ReadPrivateFile(path, 8192)
	if err != nil || json.Unmarshal(raw, &fixture) != nil {
		t.Fatal("private runtime fixture unavailable")
	}
	tmp := t.TempDir()
	write := func(name, value string) string {
		p := filepath.Join(tmp, name)
		if err := os.WriteFile(p, []byte(value), 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}
	identity := write("identity.token", "synthetic-ranking-identity-token-20261104")
	read := write("read.token", "synthetic-community-purpose-token-20261104")
	cfg := Config{Selected: []string{"community"}, Socket: socket, TokenFile: identity, UserReadSocket: socket, UserReadTokenFile: read, HostCredential: []byte("synthetic-ranking-http-host-token-20261104"), HostPublicOrigin: "https://synthetic.example", Community: StorageConfig{Component: "community", URLFile: fixture["community"].RuntimeURLFile, Database: fixture["community"].Database}}
	runtime, err := New(context.Background(), cfg)
	if err != nil {
		t.Fatal("actual community-only runtime rejected", err)
	}
	defer runtime.Close()
	mux := http.NewServeMux()
	runtime.Community.Register(mux, nil)
	direct, err := community.NewCoreRead(socket, []byte("synthetic-community-purpose-token-20261104"))
	if err != nil {
		t.Fatal(err)
	}
	defer direct.Close()
	now := time.Now().Unix()
	from := now - 30*86400
	models, err := direct.ModelBuckets(context.Background(), from, now, 0)
	if err != nil || len(models) == 0 {
		t.Fatal("populated restored Core model facts required", err)
	}
	sort.Slice(models, func(i, j int) bool {
		if models[i].Tokens != models[j].Tokens {
			return models[i].Tokens > models[j].Tokens
		}
		return models[i].ModelName < models[j].ModelName
	})
	call := func(path string) *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", path, nil))
		return w
	}
	w := call("/api/rankings?period=month")
	var ranked struct {
		Success bool
		Data    community.RankingsResponse
	}
	if json.Unmarshal(w.Body.Bytes(), &ranked) != nil || !ranked.Success || w.Code != 200 || len(ranked.Data.Models) != min(20, len(models)) {
		t.Fatalf("composed model rankings: HTTP %d", w.Code)
	}
	for i, v := range ranked.Data.Models {
		if v.ModelName != models[i].ModelName || v.TotalTokens != models[i].Tokens {
			t.Fatal("composed rankings differ from real Core")
		}
	}
	users, err := direct.UserRankings(context.Background(), from, now, 20)
	if err != nil {
		t.Fatal(err)
	}
	w = call("/api/rankings/users?period=month")
	var people struct {
		Success bool
		Data    community.UserUsageRankingsResponse
	}
	if json.Unmarshal(w.Body.Bytes(), &people) != nil || !people.Success || w.Code != 200 || people.Data.TotalTokens != users.TotalTokens || people.Data.TotalRequests != users.TotalRequests || people.Data.ParticipantCount != users.ParticipantCount || people.Data.AnonymousParticipantCount != users.AnonymousParticipantCount || len(people.Data.Users) != len(users.Users) {
		t.Fatalf("composed user rankings: HTTP %d", w.Code)
	}
	for i, v := range people.Data.Users {
		if v.TotalTokens != users.Users[i].Tokens || v.Requests != users.Users[i].Requests || v.Anonymous != users.Users[i].Anonymous || (v.Anonymous && v.Name != "") {
			t.Fatal("ranking totals or anonymity changed")
		}
	}
	combined := cfg
	combined.Selected = []string{"community", "acquisition"}
	combined.Acquisition = StorageConfig{Component: "acquisition", URLFile: fixture["acquisition"].RuntimeURLFile, Database: fixture["acquisition"].Database}
	both, err := New(context.Background(), combined)
	if err != nil {
		t.Fatal("combined runtime rejected", err)
	}
	defer both.Close()
	if both.Acquisition.UserRead == nil || both.Acquisition.ClientRead == nil {
		t.Fatal("shared reader or module-owned confirmation reader missing")
	}
	api := http.NewServeMux()
	both.Acquisition.Register(api, rankingAdmin{})
	response := httptest.NewRecorder()
	api.ServeHTTP(response, httptest.NewRequest("GET", "/api/admin/acquisition/report?from="+strconv.FormatInt(from, 10)+"&to="+strconv.FormatInt(now, 10), nil))
	if response.Code != 200 {
		t.Fatalf("composed acquisition report: HTTP %d", response.Code)
	}
	noRead := cfg
	noRead.UserReadTokenFile = ""
	noRead.UserReadSocket = ""
	offline, err := New(context.Background(), noRead)
	if err != nil {
		t.Fatal(err)
	}
	defer offline.Close()
	missing := http.NewServeMux()
	offline.Community.Register(missing, nil)
	response = httptest.NewRecorder()
	missing.ServeHTTP(response, httptest.NewRequest("GET", "/api/rankings?period=month", nil))
	if response.Code != 503 {
		t.Fatal("missing read port must remain explicit")
	}
	bad := cfg
	bad.UserReadTokenFile = identity
	if r, e := New(context.Background(), bad); e == nil {
		r.Close()
		t.Fatal("wrong-purpose credential accepted")
	}
	t.Logf("runtime rankings match restored Core: %d model leaders; %d user leaders", len(ranked.Data.Models), len(people.Data.Users))
}

func TestCoreReadConfigurationRequiresConsumer(t *testing.T) {
	for _, modules := range [][]string{nil, {"identity"}} {
		_, err := New(context.Background(), Config{Selected: modules, UserReadTokenFile: "/unused"})
		if err == nil || err.Error() != "Core read configuration requires community or acquisition" {
			t.Fatalf("unused read authority: %v", err)
		}
	}
}
