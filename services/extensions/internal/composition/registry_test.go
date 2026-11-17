// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"context"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
)

func TestRegisteredModuleSelection(t *testing.T) {
	for _, test := range []struct {
		config   string
		defaults []string
		want     []string
		invalid  bool
	}{
		{config: "hero-sms", want: []string{"hero-sms"}},
		{config: "", defaults: []string{"identity"}, want: []string{"identity"}},
		{config: "none", defaults: []string{"identity"}, want: []string{}},
		{config: " support , store ", want: []string{"support", "store"}},
		{config: "support", invalid: true},
		{config: "missing", invalid: true},
		{config: "remote-control", invalid: true},
		{config: "identity,identity", invalid: true},
	} {
		t.Run(test.config, func(t *testing.T) {
			got, err := SelectModules(test.config, test.defaults)
			if (err != nil) != test.invalid || (!test.invalid && !reflect.DeepEqual(got, test.want)) {
				t.Fatalf("selection=%v error=%v; want=%v invalid=%t", got, err, test.want, test.invalid)
			}
		})
	}
	var names []string
	for _, descriptor := range ModuleDescriptors() {
		names = append(names, descriptor.Name)
	}
	if got, err := SelectModules(strings.Join(names, ","), nil); err != nil || !reflect.DeepEqual(got, names) {
		t.Fatalf("compiled catalogue cannot be selected: %v %v", got, err)
	}
}

func TestCompositionRejectsInvalidSelectionBeforeCredentials(t *testing.T) {
	for _, test := range []struct {
		selected []string
		message  string
	}{
		{[]string{"missing"}, "unknown or duplicate"},
		{[]string{"identity", "identity"}, "unknown or duplicate"},
		{[]string{"support"}, "support requires the store module"},
	} {
		runtime, err := New(context.Background(), Config{Selected: test.selected, Socket: "/unused.sock", TokenFile: "/missing-token"})
		if runtime != nil || err == nil || !strings.Contains(err.Error(), test.message) {
			t.Fatalf("selection=%v runtime=%v error=%v", test.selected, runtime, err)
		}
	}
}

func TestModuleRegistryBindsSelectedRoutes(t *testing.T) {
	credential := []byte("synthetic_registry_host_credential_000001")
	runtime, err := New(context.Background(), Config{
		Selected:       []string{"identity"},
		Socket:         "/tmp/lmm-registry-unused.sock",
		HostCredential: credential,
		TokenFile:      paymentPrivate(t, "identity-token", "synthetic_registry_identity_credential_000001"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	if len(runtime.Modules) != 1 || runtime.Modules[0].Name() != "identity" {
		t.Fatalf("selected module order was not bound: %v", runtime.Modules)
	}
	host, err := modules.New(runtime.Modules, credential)
	if err != nil {
		t.Fatal(err)
	}
	// The real identity adapter retains its user credential check.
	// This rejected request does not need a live Core or a database.
	for _, path := range []string{"/extensions/v1/identity/self"} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		request.Header.Set("Authorization", "Bearer "+string(credential))
		response := httptest.NewRecorder()
		host.ServeHTTP(response, request)
		if response.Code != http.StatusUnauthorized {
			t.Fatalf("bound route %s status=%d body=%s", path, response.Code, response.Body.String())
		}
	}
	request := httptest.NewRequest(http.MethodGet, "/extensions/v1/remote-control/pi/sessions", nil)
	request.Header.Set("Authorization", "Bearer "+string(credential))
	response := httptest.NewRecorder()
	host.ServeHTTP(response, request)
	if response.Code != http.StatusNotFound {
		t.Fatalf("retired remote route status=%d body=%s", response.Code, response.Body.String())
	}
}
