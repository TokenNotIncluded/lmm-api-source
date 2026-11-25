package assistant_channel_git

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type authority struct{ actor access.Principal }

func (a *authority) Check(_ context.Context, token string, _ access.Account, _ access.Permission) (access.Principal, error) {
	if token != "private-fixture-session" {
		return access.Principal{}, access.ErrUnauthorized
	}
	return a.actor, nil
}
func TestAuthenticatedRegistryStagesOnlyAndRecoversGitReceipt(t *testing.T) {
	root := t.TempDir()
	git := func(args ...string) string {
		t.Helper()
		b, e := exec.Command("git", append([]string{"-C", root}, args...)...).CombinedOutput()
		if e != nil {
			t.Fatalf("git: %v %s", e, b)
		}
		return strings.TrimSpace(string(b))
	}
	git("init", "-q", "-b", "reviewed")
	git("config", "user.name", "Fixture")
	git("config", "user.email", "fixture@localhost")
	installed := filepath.Join(root, "installed.toml")
	os.WriteFile(installed, []byte("version=1\n"), 0600)
	model := filepath.Join(root, "models.toml")
	os.WriteFile(model, []byte("version=1\n"), 0600)
	git("add", "models.toml")
	git("commit", "-qm", "base")
	base := git("rev-parse", "HEAD")
	proc := filepath.Join(root, "fixture-core")
	// Stand-in only for native routing preview. The Go registry, original
	// authenticated Session, HTTP readback, Python CLI, and Git are all real.
	if e := os.WriteFile(proc, []byte(`#!/usr/bin/python3
import json,os,sys
assert sys.argv[1:]==["plan-channel-model-change"] and "LMM_ADMIN_SESSION" not in os.environ
r=json.load(sys.stdin)
def evidence(v): return {"version":{"routing":v,"pricing":1},"routing_sha256":("a" if v==1 else "b")*64,"pricing_sha256":"c"*64}
print(json.dumps({"model_config":"version=2\n","before":evidence(1),"after":evidence(2),"preview":{"channel_id":2,"candidates":[{}],"draft_bindings":[]}}))
`), 0700); e != nil {
		t.Fatal(e)
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "GET" || r.URL.Path != "/core/v1/admin/routing/configuration" || r.Header.Get("Authorization") != "Bearer private-fixture-session" {
			http.Error(w, "forbidden", 403)
			return
		}
		v := map[string]int{"routing": 1, "pricing": 1}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]any{"success": true, "data": map[string]any{"version": v, "authority": map[string]any{"mode": "file", "source": "model.config_file", "version": v, "routing_sha256": strings.Repeat("a", 64), "pricing_sha256": strings.Repeat("c", 64)}}})
	}))
	defer server.Close()
	script, e := filepath.Abs("../../../../../scripts/channel-change.py")
	if e != nil {
		t.Fatal(e)
	}
	auth := &authority{actor: access.Principal{UserID: 7, Level: 6, Account: access.Account{Kind: "personal", ID: 7}}}
	config := Config{Repo: root, ReleaseBranch: "reviewed", Core: proc, Script: script, Config: "models.toml", Instance: server.URL, Installed: installed, AllowInsecureLoopback: true}
	binding, e := Binding(config, auth)
	if e != nil {
		t.Fatal(e)
	}
	registry, e := assistant.NewAgentRegistry([]assistant.AgentToolBinding{binding})
	if e != nil {
		t.Fatal(e)
	}
	scope, e := registry.Scope(binding.Name)
	if e != nil {
		t.Fatal(e)
	}
	raw := json.RawMessage(`{"expected_version":{"routing":1,"pricing":1},"channel_id":2,"attachments":[]}`)
	in := assistant.AgentInvocation{Actor: auth.actor, Credential: "private-fixture-session", Call: assistant.AgentToolCall{OperationID: "op-001", CoreConversationID: "lmm_cv_" + strings.Repeat("1", 64), TurnID: "turn-001", ModelOperationID: "model-001", ProviderCallID: "provider-001", Scope: scope, Arguments: raw, ArgumentsSHA256: digest(raw)}}
	ctx := context.Background()
	missing, e := registry.Inspect(ctx, in)
	if e != nil || missing.State != assistant.AgentToolUnknown {
		t.Fatalf("inspect missing: %v / %v", missing, e)
	}
	staged, e := registry.Execute(ctx, in)
	if e != nil || staged.State != assistant.AgentToolSucceeded {
		t.Fatalf("stage: %v / %v", staged, e)
	}
	var result receipt
	if e = json.Unmarshal(staged.ResultJSON, &result); e != nil {
		t.Fatal(e)
	}
	if git("rev-parse", "HEAD") != base || git("rev-parse", result.Branch) != result.Commit || len(result.Commit) != 40 || result.Channel != 2 || strings.Contains(string(staged.ResultJSON), in.Credential) {
		t.Fatal("incorrect staged receipt")
	}
	again, e := registry.Execute(ctx, in)
	if e != nil || again.DomainReceiptID != staged.DomainReceiptID {
		t.Fatalf("repeat: %v", e)
	}
	recovered, e := registry.Inspect(ctx, in)
	if e != nil || recovered.DomainReceiptID != staged.DomainReceiptID {
		t.Fatalf("recover: %v", e)
	}
	body, _ := os.ReadFile(installed)
	if string(body) != "version=1\n" {
		t.Fatal("installed file was changed")
	}
	statusCheck := func(namespace string, channel uint64) ([]byte, error) {
		argv := []string{"-I", script, "assistant-status", "--namespace", namespace, "--repo", root, "--release-branch", "reviewed", "--instance", server.URL, "--allow-insecure-loopback", "--installed", installed}
		cmd := exec.Command("/usr/bin/python3", argv...)
		cmd.Env = []string{"PATH=/usr/bin:/bin", "LMM_ADMIN_SESSION=" + in.Credential}
		body, _ := json.Marshal(map[string]any{"channel_id": channel, "commit": result.Commit})
		cmd.Stdin = strings.NewReader(string(body))
		return cmd.Output()
	}
	status, err := statusCheck(OperatorNamespace(7), 2)
	if err != nil {
		t.Fatal(err)
	}
	var proof struct {
		State   string
		Channel uint64 `json:"channel_id"`
		SHA     string `json:"candidate_file_sha256"`
	}
	if json.Unmarshal(status, &proof) != nil || proof.State != "awaiting_review" || proof.Channel != 2 || proof.SHA != result.FileSHA {
		t.Fatalf("bad status proof: %s", status)
	}
	if _, e = statusCheck(OperatorNamespace(7), 3); e == nil {
		t.Fatal("wrong channel read")
	}
	if _, e = statusCheck(OperatorNamespace(8), 2); e == nil {
		t.Fatal("other operator read")
	}
	in.Call.Arguments = json.RawMessage(`{"expected_version":{"routing":1,"pricing":1},"channel_id":2,"attachments":[],"authorization":"private-leak"}`)
	in.Call.ArgumentsSHA256 = digest(in.Call.Arguments)
	if _, e = registry.Execute(ctx, in); !errors.Is(e, access.ErrInvalid) {
		t.Fatalf("unexpected authorization field accepted: %v", e)
	}
	in.Call.Arguments = raw
	in.Call.ArgumentsSHA256 = digest(raw)
	auth.actor.Level = 5
	if _, e = registry.Execute(ctx, in); !errors.Is(e, access.ErrForbidden) {
		t.Fatalf("nonroot accepted: %v", e)
	}
}

func TestAdapterOutputLimit(t *testing.T) {
	var out boundedOutput
	_, err := io.Copy(&out, strings.NewReader(strings.Repeat("x", 65537)))
	if err == nil || out.data.Len() > 65536 {
		t.Fatal("subprocess output exceeded bounded receipt")
	}
}
