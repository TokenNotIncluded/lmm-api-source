// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket_test

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_market"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func TestAssistantRemoteLostResponseReopensPostgresWithoutRedispatch(t *testing.T) {
	ctx := context.Background()
	f := toolmarket.NewAssistantLostResponseFixture(t)
	executor, err := assistant_market.New(f.Module)
	if err != nil {
		t.Fatal(err)
	}
	b := executor.Binding()
	hash := func(raw []byte) string { sum := sha256.Sum256(raw); return hex.EncodeToString(sum[:]) }
	args := json.RawMessage(`{"service_id":"service","tool_id":"tool","version_id":"version","grant_id":"grant","arguments":{}}`)
	in := assistant.AgentInvocation{Credential: "1", Actor: access.Principal{UserID: 1, Account: access.Account{Kind: "personal", ID: 1}}, Call: assistant.AgentToolCall{
		OperationID: "original-assistant-op", Arguments: args, ArgumentsSHA256: hash(args),
		Scope: assistant.AgentToolScope{Name: b.Name, Version: b.Version, SchemaSHA256: hash(b.Parameters), PriceVersion: b.PriceVersion, PriceSHA256: b.PriceSHA256, Domain: "extension", ResourceKind: "toolmarket", ResourceID: "tool", ResourceVersion: "version", GrantID: "grant", ResourceSchemaSHA256: f.Quote.RemoteDigest, ResourcePriceSHA256: f.Quote.PricingDigest},
	}}
	unknown := func(r assistant.AgentToolResult, err error) {
		t.Helper()
		if err != nil || r.State != assistant.AgentToolUnknown || r.DomainReceiptID != "" {
			t.Fatalf("assistant state=%q domain receipt=%q err=%v", r.State, r.DomainReceiptID, err)
		}
	}
	unknown(executor.Execute(ctx, in))
	f.CheckUnknown(t)

	f.Reopen(t)
	executor, err = assistant_market.New(f.Module)
	if err != nil {
		t.Fatal(err)
	}
	unknown(executor.Inspect(ctx, in))
	if n, err := f.Module.LegacyRecoverPending(ctx, 20); err != nil || n != 0 {
		t.Fatalf("deadline recovery resolved %d: %v", n, err)
	}
	unknown(executor.Execute(ctx, in))
	unknown(executor.Inspect(ctx, in))
	f.CheckUnknown(t)
}
