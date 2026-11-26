// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"testing"
)

type marketTestProvider struct{ invoked, observed int }

func (p *marketTestProvider) Invoke(context.Context, BuiltinInvocation, string) (any, error) {
	p.invoked++
	return map[string]bool{"preview": true}, nil
}
func (p *marketTestProvider) Observe(context.Context, BuiltinInvocation, string) (any, error) {
	p.observed++
	return map[string]bool{"preview": true}, nil
}

func TestRegisteredRelayToolsUseCoreAndPluginLifecycle(t *testing.T) {
	gate := &builtinGateStub{}
	dispatcher := &BuiltinDispatcher{Gate: gate}
	dispatcher.ready.Store(true)
	provider := &marketTestProvider{}
	enabled := false
	if err := dispatcher.RegisterPluginTools("ssmcpd", provider, func() bool { return enabled }); err != nil {
		t.Fatal(err)
	}
	definition, tool, err := compiledBuiltin("ssmcpd", "ssmcpd.quota")
	if err != nil {
		t.Fatal(err)
	}
	args := map[string]any{}
	in := BuiltinInvocation{Context: BuiltinContext{CallID: "call", DispatchID: "dispatch", ToolID: tool.ToolID, VersionID: definition.VersionID, GrantID: "grant", InputDigest: digest(args)}, Owner: Principal{"7", "7"}, ServiceKey: "ssmcpd", ToolName: tool.Name, Arguments: []byte("{}")}
	ctx := context.Background()
	if dispatcher.BuiltinServiceAvailable("ssmcpd") {
		t.Fatal("disabled provider is visible")
	}
	if _, err = dispatcher.InvokeBuiltin(ctx, "test-session", in); err != ErrUnavailable || provider.invoked != 0 {
		t.Fatalf("disabled dispatch: %v", err)
	}
	enabled = true
	gate.alter = true
	if _, err = dispatcher.InvokeBuiltin(ctx, "test-session", in); err != ErrDenied || provider.invoked != 0 {
		t.Fatalf("invalid Core receipt: %v", err)
	}
	gate.alter = false
	if _, err = dispatcher.InvokeBuiltin(ctx, "test-session", in); err != nil || provider.invoked != 1 {
		t.Fatal(err)
	}
	if _, err = dispatcher.QueryBuiltin(ctx, "test-session", in); err != nil || provider.invoked != 1 || provider.observed != 1 {
		t.Fatalf("lost response recovery: %v", err)
	}
	if err = dispatcher.RegisterPluginTools("ssmcpd", provider, func() bool { return true }); err == nil {
		t.Fatal("duplicate provider replaced original")
	}
}

func TestStandaloneRelayDoesNotAcquireDrawingOrWallet(t *testing.T) {
	d, err := NewPluginDispatcher(context.Background(), &builtinGateStub{})
	if err != nil || !d.LegacyBuiltinReady() {
		t.Fatalf("plugin authority not ready: %v", err)
	}
	if d.BuiltinServiceAvailable("drawing") || d.BuiltinServiceAvailable("wallet") || d.BuiltinServiceAvailable("ssmcpd") {
		t.Fatal("unregistered service advertised")
	}
	if err = d.RegisterPluginTools("ssmcpd", &marketTestProvider{}, func() bool { return true }); err != nil || !d.BuiltinServiceAvailable("ssmcpd") {
		t.Fatal("registered service unavailable", err)
	}
}
