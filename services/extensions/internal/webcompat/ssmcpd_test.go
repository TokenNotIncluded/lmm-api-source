package webcompat

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcpd"
)

func TestSSMCPDPluginUsesSessionActorAndDedicatedMeter(t *testing.T) {
	const capPath = "/api/plugins/ssmcpd/capabilities"
	const usage = "/api/plugins/ssmcpd/usage"
	token := strings.Repeat("a", 64)
	var calls int
	var replyStatus int
	var reply string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.Method != "POST" || r.URL.Path != "/internal/v1/ssmcp/quota" || r.Header.Get("Authorization") != "Bearer "+token || r.Header.Get("X-LMM-User-Credential") != "" {
			t.Error("incorrect quota request or credentials forwarded")
		}
		var body map[string]string
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || len(body) != 1 || body["user_id"] != "17" {
			t.Errorf("wrong quota owner: %v", body)
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(replyStatus)
		_, _ = w.Write([]byte(reply))
	}))
	defer upstream.Close()
	path := filepath.Join(t.TempDir(), "fixture-token")
	if err := os.WriteFile(path, []byte(token), 0600); err != nil {
		t.Fatal(err)
	}
	s, err := ssmcpd.New(ssmcpd.Config{Mode: "relay", PublicOrigin: "https://api.lmm.best", Metering: &ssmcpd.InternalConfig{Origin: upstream.URL, TokenFile: path}})
	if err != nil {
		t.Fatal(err)
	}
	if err := s.CheckCredentials([]byte(hostToken), []byte(token)); err == nil {
		t.Fatal("accepted a reused Core credential")
	}
	if err := s.CheckCredentials([]byte(hostToken)); err != nil {
		t.Fatal(err)
	}
	core := &authCore{fn: func(context.Context, string) (*pb.AuthorizeResponse, error) {
		a := sessionSelf()
		a.UserId = 17
		a.Owner.Id = 17
		a.PlatformLevel = 1
		return a, nil
	}}
	auth := newAuth(t, core)
	host, err := modules.NewHost([]modules.Module{NewSSMCPDModule(auth, s)}, []byte(hostToken))
	if err != nil {
		t.Fatal(err)
	}
	if err = host.MarkReady(); err != nil {
		t.Fatal(err)
	}
	controller := &pluginControllerFake{enabled: map[string]bool{"ssmcpd": true}}
	app := Mount(host, auth, nil, nil, AdditionalServices{Plugins: controller, SSMCPD: s})
	for _, c := range []struct {
		path, token string
		status      int
	}{{capPath, "", 401}, {capPath, hostToken, 401}, {capPath, webToken, 200}, {"/api/plugins/ssmcpd/admin/capabilities", webToken, 403}, {usage + "?user_id=2", webToken, 422}} {
		w := pluginRequest(app, "GET", c.path, "", c.token)
		if w.Code != c.status {
			t.Fatalf("%s: got %d %s", c.path, w.Code, w.Body.String())
		}
	}
	if calls != 0 {
		t.Fatal("discovery or rejected input contacted the daemon")
	}
	const valid = `{"user_id":"17","username":"alice","period":"2026-11","utc_offset_minutes":480,"reset_at":1796054400,"free_bytes":"100","paid_bytes":"20","used_bytes":"7","uploaded_bytes":"7","downloaded_bytes":"2","legacy_used_bytes":"0","limit_bytes":"120","remaining_bytes":"113"}`
	unlimited := strings.Replace(valid, `"limit_bytes":"120","remaining_bytes":"113"`, `"limit_bytes":null,"remaining_bytes":null`, 1)
	for _, c := range []struct {
		up    int
		reply string
		want  int
	}{{200, valid, 200}, {200, unlimited, 200}, {200, strings.Replace(unlimited, `,"remaining_bytes":null`, "", 1), 503}, {200, strings.Replace(unlimited, `"limit_bytes":null`, `"limit_bytes":"120"`, 1), 503}, {200, strings.Replace(valid, "\"17\"", "\"18\"", 1), 503}, {200, strings.Replace(valid, "\"113\"", "-1", 1), 503}, {404, `{"error":"unknown_user"}`, 404}, {404, "", 503}, {401, "", 503}} {
		replyStatus, reply = c.up, c.reply
		w := pluginRequest(app, "GET", usage, "", webToken)
		if w.Code != c.want {
			t.Fatalf("upstream %d: got %d %s", c.up, w.Code, w.Body.String())
		}
		if w.Code == 200 && ((reply == valid && !strings.Contains(w.Body.String(), `"remaining_bytes":"113"`)) || (reply == unlimited && !strings.Contains(w.Body.String(), `"remaining_bytes":null`))) {
			t.Fatal(w.Body.String())
		}
		if strings.Contains(w.Body.String(), token) {
			t.Fatal("exposed service credential")
		}
	}
	controller.enabled["ssmcpd"] = false
	if w := pluginRequest(app, "GET", usage, "", webToken); w.Code != 503 {
		t.Fatalf("disabled plugin returned %d", w.Code)
	}
}
