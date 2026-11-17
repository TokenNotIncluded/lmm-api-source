// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"net/http"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
)

func TestCheckinSettingsRequiresFreshRootAndSurvivesPluginDisable(t *testing.T) {
	actor := sessionSelf()
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) { return actor, nil }}
	auth := newAuth(t, core)
	host, e := modules.NewHost(nil, []byte(hostToken))
	if e != nil {
		t.Fatal(e)
	}
	if e = host.MarkReady(); e != nil {
		t.Fatal(e)
	}
	manager := &pluginControllerFake{enabled: map[string]bool{"checkins": false}}
	handler := Mount(host, auth, nil, nil, AdditionalServices{Plugins: manager})
	for _, token := range []string{"", hostToken} {
		if response := operationsRequest(handler, "GET", "/api/option/checkin", "", token); response.Code != 401 {
			t.Fatal("non-session accepted", response.Code)
		}
	}
	for _, level := range []int32{0, 5, 6} {
		actor.PlatformLevel = level
		for _, method := range []string{"GET", "PUT"} {
			before := core.calls
			response := operationsRequest(handler, method, "/api/option/checkin", "{}", webToken)
			want := http.StatusForbidden
			if level == 6 {
				want = http.StatusServiceUnavailable
			}
			if response.Code != want || core.calls != before+1 {
				t.Fatal("fresh root auth", level, method, response.Code, core.calls-before)
			}
			if level == 6 && (!strings.Contains(response.Body.String(), `"code":"CHECKIN_SETTINGS_NOT_ASSEMBLED"`) || response.Header().Get("Cache-Control") != "no-store") {
				t.Fatal("unassembled module not identified", response.Body.String())
			}
		}
	}
	if response := operationsRequest(handler, "GET", "/api/option/checkin?user_id=1", "", webToken); response.Code != http.StatusUnprocessableEntity {
		t.Fatal("query accepted", response.Code)
	}
}
