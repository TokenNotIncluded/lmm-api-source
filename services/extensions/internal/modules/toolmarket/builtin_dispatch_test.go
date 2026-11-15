package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/wallet"
	"strings"
	"testing"
)

type builtinGateStub struct {
	calls int
	alter bool
}

func (g *builtinGateStub) AuthorizeBuiltin(_ context.Context, _ string, in BuiltinAuthorizationRequest) (BuiltinAuthorization, error) {
	g.calls++
	r := BuiltinAuthorization{Owner: Principal{"7", "7"}, CallID: in.Context.CallID, DispatchID: in.Context.DispatchID, ToolID: in.Context.ToolID, VersionID: in.Context.VersionID, InputDigest: in.Context.InputDigest, InputSchemaDigest: in.InputSchemaDigest}
	if g.alter {
		r.DispatchID = "another-dispatch"
	}
	return r, nil
}
func builtinTestInvocation(t *testing.T, name string, args string) BuiltinInvocation {
	t.Helper()
	def, tool, e := compiledBuiltin("wallet", name)
	if e != nil {
		t.Fatal(e)
	}
	var parsed any
	if e = decodeStrict([]byte(args), &parsed); e != nil {
		t.Fatal(e)
	}
	return BuiltinInvocation{Context: BuiltinContext{CallID: "call", DispatchID: "dispatch", ToolID: tool.ToolID, VersionID: def.VersionID, GrantID: "grant", InputDigest: digest(parsed)}, Owner: Principal{"7", "7"}, ServiceKey: "wallet", ToolName: name, Arguments: json.RawMessage(args)}
}
func TestCompiledBuiltinCatalogIsImmutableAndSchemasValidate(t *testing.T) {
	seen := map[string]bool{}
	count := 0
	for _, d := range CompiledBuiltinDefinitions() {
		for _, tool := range d.Tools {
			count++
			if seen[tool.ToolID] {
				t.Fatal("duplicate compiled tool")
			}
			seen[tool.ToolID] = true
			if _, e := readSchema(json.RawMessage(tool.InputSchema), 0); e != nil {
				t.Fatalf("%s: %v", tool.Name, e)
			}
			if tool.PriceQuota != 0 || tool.VersionID != d.VersionID || tool.InputSchemaDigest == "" {
				t.Fatal("broken compiled basis")
			}
		}
	}
	if count != 29 {
		t.Fatalf("got %d tools", count)
	}
	a := CompiledBuiltinDefinitions()
	a[0].Tools[0].Name = "mutated"
	if CompiledBuiltinDefinitions()[0].Tools[0].Name == "mutated" {
		t.Fatal("registry aliases caller memory")
	}
}
func TestCompiledBuiltinRejectsVersionSchemaAndDigestBeforeGate(t *testing.T) {
	for _, mutate := range []func(*BuiltinInvocation){func(in *BuiltinInvocation) { in.Context.VersionID = "other" }, func(in *BuiltinInvocation) { in.Arguments = json.RawMessage(`{"payer":"attacker"}`) }, func(in *BuiltinInvocation) { in.Context.InputDigest = "other" }} {
		gate := &builtinGateStub{}
		d := &BuiltinDispatcher{Gate: gate}
		in := builtinTestInvocation(t, "wallet.balance", `{}`)
		mutate(&in)
		if _, e := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); e == nil {
			t.Fatal("substitution accepted")
		}
		if gate.calls != 0 {
			t.Fatal("invalid input reached authority")
		}
	}
}
func TestCompiledBuiltinRequiresMatchingCoreDispatchAndRealDomain(t *testing.T) {
	in := builtinTestInvocation(t, "wallet.balance", `{}`)
	gate := &builtinGateStub{alter: true}
	d := &BuiltinDispatcher{Gate: gate, Wallet: &wallet.Service{}}
	if _, e := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrDenied {
		t.Fatalf("wrong bound dispatch: %v", e)
	}
	gate.alter = false
	d.Wallet = nil
	if _, e := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrUnavailable {
		t.Fatalf("missing wallet authority: %v", e)
	}
}
func TestCompiledBuiltinQueryWithoutOriginalReferenceDoesNotInvoke(t *testing.T) {
	gate := &builtinGateStub{}
	d := &BuiltinDispatcher{Gate: gate}
	in := builtinTestInvocation(t, "wallet.transfer.create", `{"quota":10}`)
	if _, e := d.QueryBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrUnavailable {
		t.Fatalf("%v", e)
	}
	if gate.calls != 0 {
		t.Fatal("query invoked execution gate")
	}
}

type builtinWalletObserver struct {
	wallet.Authority
	prepares, queries int
	originalKey       string
	substitute        bool
}

func (a *builtinWalletObserver) Balance(context.Context, string) (wallet.Balance, error) {
	return wallet.Balance{OwnerUserID: 7, AccountID: 9, CreditsPerUSD: 500000}, nil
}
func (a *builtinWalletObserver) Prepare(context.Context, string, wallet.PrepareInput) (wallet.Transfer, error) {
	a.prepares++
	return wallet.Transfer{}, wallet.ErrUnknown
}
func (a *builtinWalletObserver) QueryTransferByRequestKey(_ context.Context, _ string, key string) (wallet.Transfer, error) {
	a.queries++
	a.originalKey = key
	t := wallet.Transfer{ID: 42, RequestKey: key, AmountCredits: 10, State: "pending"}
	if a.substitute {
		t.AmountCredits = 11
	}
	return t, nil
}

type builtinWalletLinks struct{ wallet.Records }

func (builtinWalletLinks) Token(context.Context, int64, string) (string, error) {
	return strings.Repeat("a", 64), nil
}
func TestCompiledBuiltinLostCreateResponseQueriesOriginalKey(t *testing.T) {
	authority := &builtinWalletObserver{}
	service, e := wallet.New(authority, builtinWalletLinks{}, "https://core.example.test")
	if e != nil {
		t.Fatal(e)
	}
	gate := &builtinGateStub{}
	d := &BuiltinDispatcher{Gate: gate, Wallet: service}
	in := builtinTestInvocation(t, "wallet.transfer.create", `{"quota":10}`)
	result, e := d.QueryBuiltin(context.Background(), "synthetic-personal-session", in)
	if e != nil || result.DownstreamOperationID != "42" || result.ExecutionStatus != "completed" || result.SettlementStatus != "settled" {
		t.Fatalf("%+v %v", result, e)
	}
	if authority.prepares != 0 || authority.queries != 1 || authority.originalKey != digest([]string{"builtin/v1", in.Context.CallID, in.Context.ToolID}) || gate.calls != 0 {
		t.Fatal("recovery retried or changed original operation")
	}
	authority.substitute = true
	if _, e = d.QueryBuiltin(context.Background(), "synthetic-personal-session", in); e != ErrConflict {
		t.Fatalf("replacement amount accepted: %v", e)
	}
}

func (g *builtinGateStub) ReadyBuiltins(context.Context) error { return nil }
func TestCompiledBuiltinReadinessCannotUseOnlyRegistryOrClientPresence(t *testing.T) {
	d := &BuiltinDispatcher{Gate: &builtinGateStub{}}
	if d.LegacyBuiltinReady() {
		t.Fatal("new clients advertised readiness")
	}
	if d.CheckReadiness(context.Background()) == nil || d.LegacyBuiltinReady() {
		t.Fatal("partial authority ACK advertised all builtins")
	}
}

type configuredWalletReadiness struct {
	wallet.Authority
	probes int
}

func (a *configuredWalletReadiness) Ready(context.Context) error { a.probes++; return nil }

type configuredDrawingReadiness struct {
	retiredDrawingPort
	probes int
}

func (a *configuredDrawingReadiness) Ready(context.Context) error { a.probes++; return nil }
func TestBuiltinReadinessOnlyAcknowledgesConfiguredBountyAuthority(t *testing.T) {
	authority := &configuredWalletReadiness{}
	service, err := wallet.New(authority, builtinWalletLinks{}, "https://core.example.test")
	if err != nil {
		t.Fatal(err)
	}
	drawing := &configuredDrawingReadiness{}
	gate := &builtinGateStub{}
	d := &BuiltinDispatcher{Gate: gate, Wallet: service, Drawing: drawing}
	if err := d.CheckReadiness(context.Background()); err != nil || !d.LegacyBuiltinReady() {
		t.Fatalf("wallet/drawing not ready: %v", err)
	}
	if d.Bounties != nil || authority.probes != 1 || drawing.probes != 1 {
		t.Fatal("unconfigured bounty acquired authority or configured authority not checked")
	}
	definition, tool, err := compiledBuiltin("open_source_bounties", "open_source_bounties.create_draft")
	if err != nil {
		t.Fatal(err)
	}
	args := map[string]any{"repository_url": "https://github.com/example/project", "title": "Example project", "description": strings.Repeat("d", 20), "rules": strings.Repeat("r", 20), "reward_quota": int64(1), "reward_slots": int64(1)}
	raw, err := json.Marshal(args)
	if err != nil {
		t.Fatal(err)
	}
	in := BuiltinInvocation{Context: BuiltinContext{CallID: "call", DispatchID: "dispatch", ToolID: tool.ToolID, VersionID: definition.VersionID, GrantID: "grant", InputDigest: digest(args)}, Owner: Principal{"7", "7"}, ServiceKey: "open_source_bounties", ToolName: tool.Name, Arguments: raw}
	if _, err := d.InvokeBuiltin(context.Background(), "synthetic-personal-session", in); err != ErrUnavailable || gate.calls != 0 {
		t.Fatalf("deferred bounty reached authorization or execution: %v, calls=%d", err, gate.calls)
	}
	d.Drawing = nil
	if d.CheckReadiness(context.Background()) == nil || d.LegacyBuiltinReady() {
		t.Fatal("missing drawing advertised readiness")
	}
}

func TestImageOnlyReadinessDoesNotAcquireWalletOrBounty(t *testing.T) {
	drawing := &configuredDrawingReadiness{}
	d := &BuiltinDispatcher{Gate: &builtinGateStub{}, Drawing: drawing}
	if err := d.CheckReadiness(context.Background()); err != nil || !d.LegacyBuiltinReady() {
		t.Fatal("image-only readiness", err)
	}
	if d.Wallet != nil || d.Bounties != nil || drawing.probes != 1 {
		t.Fatal("omitted authority acquired")
	}
}

type unavailableImageReadiness struct{ retiredDrawingPort }

func (*unavailableImageReadiness) Ready(context.Context) error { return ErrUnavailable }
func TestConfiguredImageFailureClearsAvailability(t *testing.T) {
	d := &BuiltinDispatcher{Gate: &builtinGateStub{}, Drawing: &configuredDrawingReadiness{}}
	if err := d.CheckReadiness(context.Background()); err != nil || !d.BuiltinServiceAvailable("drawing") {
		t.Fatal(err)
	}
	d.Drawing = &unavailableImageReadiness{}
	if err := d.CheckReadiness(context.Background()); !errors.Is(err, ErrUnavailable) || d.BuiltinServiceAvailable("drawing") || d.LegacyBuiltinReady() {
		t.Fatal("failed image advertised ready", err)
	}
}
