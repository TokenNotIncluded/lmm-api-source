// Package assistant_channel_git stages model selection changes in one host-selected
// Git repository. The tool never accepts a command, path, credential, or publish action.
package assistant_channel_git

import (
	"bytes"
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/url"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type Config struct {
	Repo                  string `toml:"repo"`
	ReleaseBranch         string `toml:"release_branch"`
	Config                string `toml:"config"`
	Core                  string `toml:"core"`
	Script                string `toml:"script"`
	Instance              string `toml:"instance"`
	Installed             string `toml:"installed"`
	LocalHTTP             bool   `toml:"local_http"`
	AllowInsecureLoopback bool   `toml:"allow_insecure_loopback"`
}

func (c Config) Validate() error {
	for _, path := range []string{c.Repo, c.Core, c.Script, c.Installed} {
		if !filepath.IsAbs(path) || filepath.Clean(path) != path {
			return errors.New("channel Git configuration requires absolute host paths")
		}
	}
	if c.Config == "" || filepath.IsAbs(c.Config) || filepath.Clean(c.Config) != c.Config || c.Config == ".." || strings.HasPrefix(c.Config, "../") || strings.HasPrefix(c.Config, "-") || strings.HasPrefix(c.Config, ":") {
		return errors.New("invalid tracked model configuration path")
	}
	if c.ReleaseBranch == "" || strings.HasPrefix(c.ReleaseBranch, "-") || strings.HasPrefix(c.ReleaseBranch, "ai/channel/") {
		return errors.New("invalid reviewed branch")
	}
	u, err := url.Parse(c.Instance)
	if err != nil || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return errors.New("invalid channel Git instance origin")
	}
	if u.Scheme != "https" && !(u.Scheme == "http" && c.AllowInsecureLoopback && net.ParseIP(u.Hostname()).IsLoopback()) {
		return errors.New("channel Git instance requires HTTPS or explicitly configured literal loopback")
	}
	return nil
}

func digest(data []byte) string { s := sha256.Sum256(data); return hex.EncodeToString(s[:]) }
func OperatorNamespace(userID int64) string {
	return "ai/channel/" + digest([]byte("lmm-channel-root:" + strconv.FormatInt(userID, 10)))[:32] + "/"
}

//go:embed registry.json
var manifest []byte

type descriptor struct {
	Name         string `json:"name"`
	Description  string `json:"description"`
	Version      string `json:"version"`
	Schema       string `json:"schema_json"`
	SHA          string `json:"schema_sha256"`
	PriceSHA     string `json:"price_sha256"`
	PriceVersion string `json:"price_version"`
}

type tool struct {
	cfg  Config
	auth access.Authority
	desc descriptor
	run  func(context.Context, Config, string, string, string, string, []byte) ([]byte, error)
}

func Binding(cfg Config, auth access.Authority) (assistant.AgentToolBinding, error) {
	if err := cfg.Validate(); err != nil {
		return assistant.AgentToolBinding{}, err
	}
	if access.Missing(auth) {
		return assistant.AgentToolBinding{}, access.ErrUnavailable
	}
	var catalog struct {
		Tools []descriptor `json:"tools"`
	}
	if json.Unmarshal(manifest, &catalog) != nil || len(catalog.Tools) != 1 {
		return assistant.AgentToolBinding{}, access.ErrInvalid
	}
	d := catalog.Tools[0]
	if digest([]byte(d.Schema)) != d.SHA || d.PriceSHA != assistant.FreeAgentPriceSHA256() {
		return assistant.AgentToolBinding{}, access.ErrInvalid
	}
	return assistant.AgentToolBinding{Name: d.Name, Description: d.Description, Version: d.Version, Effect: "server_guarded",
		Access: "root", PriceVersion: d.PriceVersion, PriceSHA256: d.PriceSHA, Parameters: json.RawMessage(d.Schema), Executor: &tool{cfg, auth, d, invoke}}, nil
}

type boundedOutput struct{ data bytes.Buffer }

func (b *boundedOutput) Write(p []byte) (int, error) {
	if len(p) > (64<<10)-b.data.Len() {
		return 0, errors.New("channel proposal output exceeds limit")
	}
	return b.data.Write(p)
}

func invoke(ctx context.Context, c Config, action, namespace, operation, token string, input []byte) ([]byte, error) {
	ctx, cancel := context.WithTimeout(ctx, 45*time.Second)
	defer cancel()
	// Every option comes from trusted host config or a hashed journal identity.
	args := []string{"-I", c.Script, action, "--namespace", namespace, "--repo", c.Repo, "--release-branch", c.ReleaseBranch, "--instance", c.Instance, "--operation", operation, "--config", c.Config, "--core", c.Core}
	if c.LocalHTTP {
		args = append(args, "--local-http")
	}
	if c.AllowInsecureLoopback {
		args = append(args, "--allow-insecure-loopback")
	}
	cmd := exec.CommandContext(ctx, "/usr/bin/python3", args...)
	cmd.Stdin = bytes.NewReader(input)
	output := new(boundedOutput)
	cmd.Stdout = output
	cmd.Stderr = io.Discard
	cmd.Env = []string{"PATH=/usr/bin:/bin", "LANG=C.UTF-8", "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_GLOBAL=/dev/null",
		"GIT_CONFIG_COUNT=1", "GIT_CONFIG_KEY_0=core.hooksPath", "GIT_CONFIG_VALUE_0=/dev/null", "GIT_TERMINAL_PROMPT=0",
		"GIT_AUTHOR_NAME=LMM channel assistant", "GIT_AUTHOR_EMAIL=assistant@localhost", "GIT_COMMITTER_NAME=LMM channel assistant", "GIT_COMMITTER_EMAIL=assistant@localhost", "LMM_ADMIN_SESSION=" + token}
	if err := cmd.Run(); err != nil {
		return nil, assistant.ErrDelegationUnknown
	}
	return output.data.Bytes(), nil
}

func (t *tool) authorize(ctx context.Context, in assistant.AgentInvocation) error {
	p, err := t.auth.Check(ctx, in.Credential, access.Account{}, access.Read)
	if err != nil {
		return err
	}
	if p.Level != 6 || p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) || p.UserID != in.Actor.UserID || p.Account != in.Actor.Account {
		return access.ErrForbidden
	}
	return nil
}

func (t *tool) Execute(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return t.dispatch(ctx, in, "assistant-stage")
}
func (t *tool) Inspect(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return t.dispatch(ctx, in, "assistant-inspect")
}

type version struct {
	Routing uint64 `json:"routing"`
	Pricing uint64 `json:"pricing"`
}
type evidence struct {
	Version version `json:"version"`
	Routing string  `json:"routing_sha256"`
	Pricing string  `json:"pricing_sha256"`
}
type receipt struct {
	State    string   `json:"state"`
	Branch   string   `json:"branch,omitempty"`
	Commit   string   `json:"commit,omitempty"`
	Channel  uint64   `json:"channel_id,omitempty"`
	FileSHA  string   `json:"candidate_file_sha256,omitempty"`
	Priced   int      `json:"priced_bindings"`
	Draft    int      `json:"draft_binding_count"`
	Before   evidence `json:"before"`
	Expected evidence `json:"expected"`
}

func lowerHex(value string, size int) bool {
	_, err := hex.DecodeString(value)
	return len(value) == size && err == nil && strings.ToLower(value) == value
}
func (e evidence) valid() bool {
	return e.Version.Routing > 0 && e.Version.Pricing > 0 && lowerHex(e.Routing, 64) && lowerHex(e.Pricing, 64)
}

func (t *tool) dispatch(ctx context.Context, in assistant.AgentInvocation, action string) (assistant.AgentToolResult, error) {
	scope := in.Call.Scope
	if len(in.Call.Arguments) > 64<<10 || !access.ID(in.Call.OperationID) || digest(in.Call.Arguments) != in.Call.ArgumentsSHA256 || scope.Name != t.desc.Name || scope.Version != t.desc.Version || scope.SchemaSHA256 != t.desc.SHA || scope.Domain != "extension" || scope.PriceVersion != t.desc.PriceVersion || scope.PriceSHA256 != t.desc.PriceSHA || scope.ResourceKind != "" || scope.ResourceID != "" || scope.GrantID != "" || scope.ResourceVersion != "" || scope.ResourceSchemaSHA256 != "" || scope.ResourcePriceSHA256 != "" {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	if err := t.authorize(ctx, in); err != nil {
		return assistant.AgentToolResult{}, err
	}
	if _, err := toolmarket.ValidateLegacyArguments(json.RawMessage(t.desc.Schema), in.Call.Arguments); err != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	var input struct {
		Channel uint64 `json:"channel_id"`
	}
	if json.Unmarshal(in.Call.Arguments, &input) != nil || input.Channel == 0 {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	ns := OperatorNamespace(in.Actor.UserID)
	op := digest([]byte(in.Call.OperationID))
	raw, err := t.run(ctx, t.cfg, action, ns, op, in.Credential, in.Call.Arguments)
	if err != nil {
		return assistant.AgentToolResult{}, assistant.ErrDelegationUnknown
	}
	if err = t.authorize(ctx, in); err != nil {
		return assistant.AgentToolResult{}, err
	}
	var result receipt
	if json.Unmarshal(raw, &result) != nil {
		return assistant.AgentToolResult{}, assistant.ErrDelegationUnknown
	}
	switch result.State {
	case "unknown":
		return assistant.AgentResult(in.Call, assistant.AgentToolUnknown, map[string]string{"state": "unknown"}, "")
	case "rejected":
		return assistant.AgentResult(in.Call, assistant.AgentToolFailed, map[string]string{"state": "rejected"}, in.Call.OperationID)
	case "staged":
		if result.Channel != input.Channel || result.Branch != ns+op || !lowerHex(result.FileSHA, 64) || (!lowerHex(result.Commit, 40) && !lowerHex(result.Commit, 64)) || result.Priced < 0 || result.Draft < 0 || !result.Before.valid() || !result.Expected.valid() {
			return assistant.AgentToolResult{}, assistant.ErrDelegationUnknown
		}
		return assistant.AgentResult(in.Call, assistant.AgentToolSucceeded, result, result.Commit)
	}
	return assistant.AgentToolResult{}, assistant.ErrDelegationUnknown
}
