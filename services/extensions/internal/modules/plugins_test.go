package modules

import (
	"bytes"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

type pluginTestModule struct{ name, version string }

func (m pluginTestModule) Name() string    { return m.name }
func (m pluginTestModule) Version() string { return m.version }
func (m pluginTestModule) Handler() http.Handler {
	return http.HandlerFunc(func(http.ResponseWriter, *http.Request) {})
}

type pluginTestReloader struct {
	last  []string
	calls int
	err   error
}

func TestPluginCatalogIncludesUnconfiguredPackages(t *testing.T) {
	registered := []Module{pluginTestModule{"alpha", "1.0.0"}}
	catalog := []Descriptor{
		{Name: "alpha", Version: "1.0.0", NavigationPaths: []string{"/custom-alpha"}},
		{Name: "beta", Version: "2.0.0", Requires: []string{"alpha"}, NavigationPaths: []string{"/custom-beta"}},
	}
	host, err := NewHost(registered, []byte(strings.Repeat("catalog-test-", 4)))
	if err != nil {
		t.Fatal(err)
	}
	manager, err := NewPluginManager(host, registered, "", catalog...)
	if err != nil {
		t.Fatal(err)
	}
	info := manager.List()
	if len(info) != 2 || !info[0].Available || !info[0].Enabled || info[1].Available || info[1].Installed || info[1].Enabled {
		t.Fatalf("catalog=%+v", info)
	}
	if !reflect.DeepEqual(info[1].Requires, []string{"alpha"}) || !reflect.DeepEqual(info[1].NavigationPaths, []string{"/custom-beta"}) {
		t.Fatalf("declaration lost: %+v", info[1])
	}
	request := PluginInstallRequest{Name: info[1].Name, Version: info[1].Version, APIVersion: info[1].APIVersion, RegistrationSHA256: info[1].RegistrationSHA256}
	if _, err := manager.Install(request); !errors.Is(err, ErrPluginUnavailable) {
		t.Fatalf("unconfigured install=%v", err)
	}
	if _, err := manager.Enable("beta"); !errors.Is(err, ErrPluginUnavailable) {
		t.Fatalf("unconfigured enable=%v", err)
	}
	// Neither the caller's declarations nor an API snapshot can mutate routing
	// dependencies retained by the manager.
	catalog[0].NavigationPaths[0] = "/changed"
	info[1].Requires[0] = "changed"
	if got := manager.List(); got[0].NavigationPaths[0] != "/custom-alpha" || got[1].Requires[0] != "alpha" {
		t.Fatalf("catalog aliased caller memory: %+v", got)
	}
}

func TestPluginDependenciesKeepRoutesAndPersistedStateConsistent(t *testing.T) {
	registered := []Module{pluginTestModule{"store", "1.0.0"}, pluginTestModule{"support", "1.0.0"}}
	catalog := []Descriptor{{Name: "store", Version: "1.0.0"}, {Name: "support", Version: "1.0.0", Requires: []string{"store"}}}
	credential := []byte(strings.Repeat("dependency-test-", 3))
	host, err := NewHost(registered, credential)
	if err != nil {
		t.Fatal(err)
	}
	if err := host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	state := filepath.Join(dir, "plugins.json")
	manager, err := NewPluginManager(host, registered, state, catalog...)
	if err != nil {
		t.Fatal(err)
	}
	before, err := os.ReadFile(state)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Disable("store"); !errors.Is(err, ErrPluginDependency) {
		t.Fatalf("disabled active dependency: %v", err)
	}
	request := httptest.NewRequest(http.MethodGet, "/extensions/v1/store/", nil)
	request.Header.Set("Authorization", "Bearer "+string(credential))
	response := httptest.NewRecorder()
	host.ServeHTTP(response, request)
	after, err := os.ReadFile(state)
	if err != nil || !bytes.Equal(before, after) || response.Code != http.StatusOK || !manager.IsEnabled("support") {
		t.Fatalf("rejected change altered routes or state: status=%d err=%v", response.Code, err)
	}
	for _, name := range []string{"support", "store"} {
		if _, err := manager.Disable(name); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := manager.Enable("support"); !errors.Is(err, ErrPluginDependency) || manager.IsEnabled("support") {
		t.Fatalf("enabled without dependency: %v", err)
	}
	restarted, err := NewPluginManager(host, registered, state, catalog...)
	if err != nil {
		t.Fatal(err)
	}
	if restarted.IsEnabled("store") || restarted.IsEnabled("support") {
		t.Fatal("restart enabled disabled packages")
	}
	for _, name := range []string{"store", "support"} {
		if _, err := restarted.Enable(name); err != nil {
			t.Fatal(err)
		}
	}
	if !restarted.IsEnabled("support") {
		t.Fatal("dependency-ordered enable failed")
	}
}

func TestPluginCatalogRejectsUndeclaredOrMismatchedRuntimeModule(t *testing.T) {
	for _, module := range []Module{pluginTestModule{"missing", "1.0.0"}, pluginTestModule{"alpha", "2.0.0"}} {
		_, err := NewPluginManager(&pluginTestReloader{}, []Module{module}, "", Descriptor{Name: "alpha", Version: "1.0.0"})
		if !errors.Is(err, ErrPluginManifest) {
			t.Fatalf("module=%v error=%v", module, err)
		}
	}
}

func (r *pluginTestReloader) ReloadModules(names []string) error {
	r.calls++
	if r.err != nil {
		return r.err
	}
	r.last = append([]string(nil), names...)
	return nil
}

func TestPluginManagerLifecyclePersistsAndHotReloads(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	state := filepath.Join(dir, "plugins.json")
	reloader := &pluginTestReloader{}
	manager, err := NewPluginManager(reloader, []Module{
		pluginTestModule{"zeta", "2.1.0"}, pluginTestModule{"alpha", "1.0.0"},
	}, state)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(reloader.last, []string{"alpha", "zeta"}) {
		t.Fatalf("initial reload=%v", reloader.last)
	}
	info := manager.List()
	if len(info) != 2 || info[0].Name != "alpha" || !info[0].Installed || !info[0].Enabled {
		t.Fatalf("inventory=%v", info)
	}
	if _, err := manager.Disable("alpha"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(reloader.last, []string{"zeta"}) {
		t.Fatalf("disable reload=%v", reloader.last)
	}
	if _, err := manager.Disable("alpha"); !errors.Is(err, ErrPluginAlreadyOff) {
		t.Fatalf("second disable err=%v", err)
	}
	if _, err := manager.Uninstall("alpha"); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Enable("alpha"); !errors.Is(err, ErrPluginNotInstalled) {
		t.Fatalf("enable uninstalled err=%v", err)
	}
	alpha := manager.List()[0]
	if _, err := manager.Install(PluginInstallRequest{Name: alpha.Name, Version: alpha.Version, APIVersion: alpha.APIVersion, RegistrationSHA256: alpha.RegistrationSHA256}); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Enable("alpha"); err != nil {
		t.Fatal(err)
	}
	reloaded, err := NewPluginManager(&pluginTestReloader{}, []Module{
		pluginTestModule{"zeta", "2.1.0"}, pluginTestModule{"alpha", "1.0.0"},
	}, state)
	if err != nil {
		t.Fatal(err)
	}
	for _, got := range reloaded.List() {
		if got.Name == "alpha" && (!got.Installed || !got.Enabled) {
			t.Fatalf("persisted alpha=%+v", got)
		}
	}
}

func TestPluginManagerRejectsUnknownOrMismatchedInstall(t *testing.T) {
	reloader := &pluginTestReloader{}
	manager, err := NewPluginManager(reloader, []Module{pluginTestModule{"alpha", "1.0.0"}}, "")
	if err != nil {
		t.Fatal(err)
	}
	for _, request := range []PluginInstallRequest{
		{Name: "missing", Version: "1.0.0", APIVersion: 1, RegistrationSHA256: strings.Repeat("a", 64)},
		{Name: "alpha", Version: "2.0.0", APIVersion: 1, RegistrationSHA256: RegistrationSHA256("alpha", "1.0.0")},
		{Name: "alpha", Version: "1.0.0", APIVersion: 2, RegistrationSHA256: RegistrationSHA256("alpha", "1.0.0")},
	} {
		if _, err := manager.Install(request); !errors.Is(err, ErrPluginNotFound) && !errors.Is(err, ErrPluginManifest) {
			t.Fatalf("request=%+v err=%v", request, err)
		}
	}
}

func TestPluginManagerInstallPersistenceFailurePreservesStateAndCanRetry(t *testing.T) {
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	statePath := filepath.Join(dir, "plugins.json")
	manager, err := NewPluginManager(&pluginTestReloader{}, []Module{
		pluginTestModule{"alpha", "1.0.0"},
	}, statePath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Disable("alpha"); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Uninstall("alpha"); err != nil {
		t.Fatal(err)
	}

	before := manager.List()
	if len(before) != 1 || before[0].Installed || before[0].Enabled || manager.IsEnabled("alpha") {
		t.Fatalf("unexpected initial state: list=%+v enabled=%t", before, manager.IsEnabled("alpha"))
	}
	persisted, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(statePath); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(statePath, 0700); err != nil {
		t.Fatal(err)
	}

	request := PluginInstallRequest{
		Name: "alpha", Version: before[0].Version, APIVersion: before[0].APIVersion,
		RegistrationSHA256: before[0].RegistrationSHA256,
	}
	for attempt := 1; attempt <= 2; attempt++ {
		if _, err := manager.Install(request); err == nil {
			t.Fatalf("attempt %d unexpectedly succeeded with an invalid state path", attempt)
		}
		if got := manager.List(); !reflect.DeepEqual(got, before) {
			t.Fatalf("attempt %d changed plugin state: got=%+v want=%+v", attempt, got, before)
		}
		if manager.IsEnabled("alpha") {
			t.Fatalf("attempt %d changed enabled state", attempt)
		}
	}

	if err := os.Remove(statePath); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(statePath, persisted, 0600); err != nil {
		t.Fatal(err)
	}
	installed, err := manager.Install(request)
	if err != nil {
		t.Fatalf("retry after restoring state file: %v", err)
	}
	if !installed.Installed || installed.Enabled || manager.IsEnabled("alpha") {
		t.Fatalf("unexpected successful retry result: %+v", installed)
	}

	reloaded, err := NewPluginManager(&pluginTestReloader{}, []Module{
		pluginTestModule{"alpha", "1.0.0"},
	}, statePath)
	if err != nil {
		t.Fatal(err)
	}
	if got := reloaded.List(); len(got) != 1 || !got[0].Installed || got[0].Enabled {
		t.Fatalf("persisted retry state=%+v", got)
	}
}

func TestRouterReloadDisablesNewModuleRequests(t *testing.T) {
	credential := []byte(strings.Repeat("router-test-", 4))
	router, err := NewRouter([]Module{pluginTestModule{"alpha", "1.0.0"}, pluginTestModule{"zeta", "1.0.0"}}, credential)
	if err != nil {
		t.Fatal(err)
	}
	if err := router.Reload([]string{"alpha"}); err != nil {
		t.Fatal(err)
	}
	request := func(path string) int {
		r := httptest.NewRequest(http.MethodGet, path, nil)
		r.Header.Set("Authorization", "Bearer "+string(credential))
		w := httptest.NewRecorder()
		router.ServeHTTP(w, r)
		return w.Code
	}
	if got := request("/extensions/v1/alpha/"); got != http.StatusOK {
		t.Fatalf("enabled status=%d", got)
	}
	if got := request("/extensions/v1/zeta/"); got != http.StatusNotFound {
		t.Fatalf("disabled status=%d", got)
	}
}

func TestPluginManagerMutationPersistenceFailurePreservesStateAndCanRetry(t *testing.T) {
	// @context-fold begin plugin-mutation-persistence-recovery
	for _, test := range []struct {
		name      string
		enabled   bool
		installed bool
		operation func(*PluginManager) (PluginInfo, error)
		want      PluginInfo
	}{
		{"enable", false, true, func(m *PluginManager) (PluginInfo, error) { return m.Enable("alpha") }, PluginInfo{Installed: true, Enabled: true}},
		{"disable", true, true, func(m *PluginManager) (PluginInfo, error) { return m.Disable("alpha") }, PluginInfo{Installed: true, Enabled: false}},
		{"uninstall", false, true, func(m *PluginManager) (PluginInfo, error) { return m.Uninstall("alpha") }, PluginInfo{Installed: false, Enabled: false}},
		{"uninstall-again", false, false, func(m *PluginManager) (PluginInfo, error) { return m.Uninstall("alpha") }, PluginInfo{Installed: false, Enabled: false}},
	} {
		t.Run(test.name, func(t *testing.T) {
			dir := t.TempDir()
			if err := os.Chmod(dir, 0700); err != nil {
				t.Fatal(err)
			}
			statePath := filepath.Join(dir, "plugins.json")
			registered := []Module{pluginTestModule{"alpha", "1.0.0"}}
			credential := []byte(strings.Repeat("router-test-", 4))
			host, err := NewHost(registered, credential)
			if err != nil {
				t.Fatal(err)
			}
			if err := host.MarkReady(); err != nil {
				t.Fatal(err)
			}
			manager, err := NewPluginManager(host, registered, statePath)
			if err != nil {
				t.Fatal(err)
			}
			// Make the initial state durable, even for the already-enabled case.
			if _, err := manager.Disable("alpha"); err != nil {
				t.Fatal(err)
			}
			if test.enabled {
				if _, err := manager.Enable("alpha"); err != nil {
					t.Fatal(err)
				}
			}
			if !test.installed {
				if _, err := manager.Uninstall("alpha"); err != nil {
					t.Fatal(err)
				}
			}
			requestStatus := func() int {
				request := httptest.NewRequest(http.MethodGet, "/extensions/v1/alpha/", nil)
				request.Header.Set("Authorization", "Bearer "+string(credential))
				response := httptest.NewRecorder()
				host.ServeHTTP(response, request)
				return response.Code
			}
			checkPersisted := func() {
				t.Helper()
				restarted, err := NewPluginManager(&pluginTestReloader{}, registered, statePath)
				if err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(restarted.List(), manager.List()) {
					t.Fatalf("restart=%+v running=%+v", restarted.List(), manager.List())
				}
			}
			checkPersisted()
			before := manager.List()
			beforeStatus := requestStatus()
			persisted, err := os.ReadFile(statePath)
			if err != nil {
				t.Fatal(err)
			}
			// Rejected directory permissions fail persistence without replacing the file.
			if err := os.Chmod(dir, 0755); err != nil {
				t.Fatal(err)
			}
			for attempt := 0; attempt < 2; attempt++ {
				if _, err := test.operation(manager); !errors.Is(err, ErrPluginState) {
					t.Fatalf("operation error=%v", err)
				}
				if got := manager.List(); !reflect.DeepEqual(got, before) {
					t.Fatalf("failed operation changed state: got=%+v want=%+v", got, before)
				}
				if got := requestStatus(); got != beforeStatus {
					t.Fatalf("failed operation changed route: got=%d want=%d", got, beforeStatus)
				}
				got, err := os.ReadFile(statePath)
				if err != nil || !bytes.Equal(got, persisted) {
					t.Fatalf("failed operation changed persisted state: err=%v", err)
				}
			}
			if err := os.Chmod(dir, 0700); err != nil {
				t.Fatal(err)
			}
			info, err := test.operation(manager)
			if err != nil {
				t.Fatal(err)
			}
			if info.Installed != test.want.Installed || info.Enabled != test.want.Enabled || manager.IsEnabled("alpha") != test.want.Enabled {
				t.Fatalf("retry state=%+v want=%+v", info, test.want)
			}
			wantStatus := http.StatusNotFound
			if test.want.Enabled {
				wantStatus = http.StatusOK
			}
			if got := requestStatus(); got != wantStatus {
				t.Fatalf("retry route=%d want=%d", got, wantStatus)
			}
			checkPersisted()
		})
	}
	// @context-fold end plugin-mutation-persistence-recovery
}

func TestPluginManagerReloadReappliesCurrentSnapshotWithoutChangingPersistence(t *testing.T) {
	// @context-fold begin plugin-reload-snapshot-persistence
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	statePath := filepath.Join(dir, "plugins.json")
	registered := []Module{pluginTestModule{"alpha", "1.0.0"}}
	credential := []byte(strings.Repeat("router-test-", 4))
	host, err := NewHost(registered, credential)
	if err != nil {
		t.Fatal(err)
	}
	manager, err := NewPluginManager(host, registered, statePath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Disable("alpha"); err != nil {
		t.Fatal(err)
	}
	persisted, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatal(err)
	}
	before := manager.List()
	if err := host.ReloadModules([]string{"alpha"}); err != nil {
		t.Fatal(err)
	}
	status := func() int {
		request := httptest.NewRequest(http.MethodGet, "/extensions/v1/alpha/", nil)
		request.Header.Set("Authorization", "Bearer "+string(credential))
		response := httptest.NewRecorder()
		host.router.ServeHTTP(response, request)
		return response.Code
	}
	if got := status(); got != http.StatusOK {
		t.Fatalf("fixture did not replace route: %d", got)
	}
	if err := manager.Reload(); err != nil {
		t.Fatal(err)
	}
	if got := status(); got != http.StatusNotFound {
		t.Fatalf("reload did not reapply disabled snapshot: %d", got)
	}
	if !reflect.DeepEqual(manager.List(), before) {
		t.Fatalf("reload changed inventory: %+v", manager.List())
	}
	got, err := os.ReadFile(statePath)
	if err != nil || !bytes.Equal(got, persisted) {
		t.Fatalf("reload changed persisted state: err=%v", err)
	}
	// @context-fold end plugin-reload-snapshot-persistence
}

func TestPluginManagerReloadFailurePreservesStateAndCanRetry(t *testing.T) {
	// @context-fold begin plugin-reloader-failure-recovery
	for _, operation := range []string{"enable", "disable", "reload"} {
		t.Run(operation, func(t *testing.T) {
			dir := t.TempDir()
			if err := os.Chmod(dir, 0700); err != nil {
				t.Fatal(err)
			}
			statePath := filepath.Join(dir, "plugins.json")
			reloader := &pluginTestReloader{}
			manager, err := NewPluginManager(reloader, []Module{pluginTestModule{"alpha", "1.0.0"}}, statePath)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := manager.Disable("alpha"); err != nil {
				t.Fatal(err)
			}
			if operation == "disable" {
				if _, err := manager.Enable("alpha"); err != nil {
					t.Fatal(err)
				}
			}
			change := func() error {
				switch operation {
				case "enable":
					_, err := manager.Enable("alpha")
					return err
				case "disable":
					_, err := manager.Disable("alpha")
					return err
				default:
					return manager.Reload()
				}
			}
			before := manager.List()
			beforeRoutes := append([]string(nil), reloader.last...)
			persisted, err := os.ReadFile(statePath)
			if err != nil {
				t.Fatal(err)
			}
			reloader.err = errors.New("test reload failure")
			if err := change(); !errors.Is(err, reloader.err) {
				t.Fatalf("reload failure=%v", err)
			}
			if !reflect.DeepEqual(manager.List(), before) {
				t.Fatalf("reload failure changed inventory: %+v", manager.List())
			}
			if !reflect.DeepEqual(reloader.last, beforeRoutes) {
				t.Fatalf("reload failure changed routes: got=%v want=%v", reloader.last, beforeRoutes)
			}
			got, err := os.ReadFile(statePath)
			if err != nil || !bytes.Equal(got, persisted) {
				t.Fatalf("reload failure changed persisted state: err=%v", err)
			}
			reloader.err = nil
			if err := change(); err != nil {
				t.Fatalf("retry: %v", err)
			}
			restarted, err := NewPluginManager(&pluginTestReloader{}, []Module{pluginTestModule{"alpha", "1.0.0"}}, statePath)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(restarted.List(), manager.List()) {
				t.Fatalf("restart=%+v running=%+v", restarted.List(), manager.List())
			}
		})
	}
	// @context-fold end plugin-reloader-failure-recovery
}
