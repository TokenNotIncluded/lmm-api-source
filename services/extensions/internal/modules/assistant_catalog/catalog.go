// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Package assistant_catalog is a read-only adapter to installed store and bounty
// services. It accepts no arbitrary operation, SQL, URL, account or money command.
package assistant_catalog

import (
	"bytes"
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/redaction"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

//go:embed definitions.json
var definitionsJSON []byte

type definition struct {
	Name         string `json:"name"`
	Description  string `json:"description"`
	Version      string `json:"version"`
	Effect       string `json:"effect"`
	Access       string `json:"access"`
	SchemaJSON   string `json:"schema_json"`
	SchemaSHA256 string `json:"schema_sha256"`
}
type storeReader interface {
	CatalogueForViewer(context.Context, string, int64, store.CatalogueQuery) ([]map[string]any, error)
	ProductForViewer(context.Context, string, int64, string) (map[string]any, error)
}
type bountyReader interface {
	Config(context.Context) (bounties.FeeConfig, error)
	BoardFor(context.Context, string, int, int) (map[string]any, error)
	Detail(context.Context, string, string) (map[string]any, error)
	BuiltinInvoke(context.Context, bounties.Invocation) (bounties.Result, error)
	DisputesForViewer(context.Context, string, bool, string, int) ([]map[string]any, error)
}
type Executor struct {
	auth        access.Authority
	store       storeReader
	bounties    bountyReader
	definitions []definition
}
type tool struct {
	executor   *Executor
	definition definition
}

// New accepts only real composed domain services, not model-selected callbacks.
// An absent optional module has no advertised bindings. The host establishes
// each domain's normal readiness before composition, as for its HTTP routes.
func New(auth access.Authority, products *store.Service, bounty *bounties.Service) (*Executor, error) {
	if access.Missing(auth) {
		return nil, access.ErrInvalid
	}
	e := &Executor{auth: auth}
	if products != nil {
		e.store = products
	}
	if bounty != nil {
		e.bounties = bounty
	}
	if json.Unmarshal(definitionsJSON, &e.definitions) != nil || len(e.definitions) != 4 {
		return nil, access.ErrUnavailable
	}
	seen := map[string]bool{}
	for _, d := range e.definitions {
		if seen[d.Name] {
			return nil, access.ErrUnavailable
		}
		seen[d.Name] = true
		if d.Version != assistant.FixedMainToolVersion || d.Effect != "read_only" || d.Access != "user" || digest([]byte(d.SchemaJSON)) != d.SchemaSHA256 {
			return nil, access.ErrUnavailable
		}
		switch d.Name {
		case "get_store_products", "get_store_product", "get_bounty_guide", "get_bounty_data":
		default:
			return nil, access.ErrUnavailable
		}
	}
	return e, nil
}
func (e *Executor) Bindings() []assistant.AgentToolBinding {
	if e == nil {
		return nil
	}
	out := []assistant.AgentToolBinding{}
	for _, d := range e.definitions {
		if (d.Name == "get_store_products" || d.Name == "get_store_product") && access.Missing(e.store) {
			continue
		}
		if (d.Name == "get_bounty_guide" || d.Name == "get_bounty_data") && access.Missing(e.bounties) {
			continue
		}
		out = append(out, assistant.AgentToolBinding{Name: d.Name, Description: d.Description, Version: d.Version, Effect: d.Effect, Access: d.Access, PriceVersion: assistant.BuiltinToolPriceVersion, PriceSHA256: assistant.FreeAgentPriceSHA256(), Parameters: json.RawMessage(d.SchemaJSON), Executor: &tool{e, d}})
	}
	return out
}
func digest(b []byte) string { h := sha256.Sum256(b); return hex.EncodeToString(h[:]) }
func (t *tool) Execute(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return t.read(ctx, in)
}
func (t *tool) Inspect(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	return t.read(ctx, in)
}
func (t *tool) principal(ctx context.Context, in assistant.AgentInvocation) (access.Principal, error) {
	p, err := t.executor.auth.Check(ctx, in.Credential, access.Account{}, access.Read)
	if err != nil {
		return p, err
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) || p.UserID != in.Actor.UserID || p.Account != in.Actor.Account {
		return p, access.ErrForbidden
	}
	return p, nil
}
func (t *tool) read(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	if t == nil || t.executor == nil || access.Missing(t.executor.auth) || in.Credential == "" || !access.ID(in.Call.OperationID) || digest(in.Call.Arguments) != in.Call.ArgumentsSHA256 {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	d, s := t.definition, in.Call.Scope
	if s.Name != d.Name || s.Domain != "extension" || s.Version != d.Version || s.SchemaSHA256 != d.SchemaSHA256 || s.PriceVersion != assistant.BuiltinToolPriceVersion || s.PriceSHA256 != assistant.FreeAgentPriceSHA256() || s.ResourceKind != "" || s.ResourceID != "" || s.ResourceVersion != "" || s.GrantID != "" || s.ResourceSchemaSHA256 != "" || s.ResourcePriceSHA256 != "" {
		return assistant.AgentToolResult{}, access.ErrConflict
	}
	if _, err := toolmarket.ValidateLegacyArguments(json.RawMessage(d.SchemaJSON), in.Call.Arguments); err != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	p, err := t.principal(ctx, in)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	var value any
	switch d.Name {
	case "get_store_products", "get_store_product":
		value, err = t.products(ctx, in, p)
	case "get_bounty_guide", "get_bounty_data":
		value, err = t.bounty(ctx, in, p)
	default:
		err = access.ErrInvalid
	}
	if err != nil {
		return assistant.AgentToolResult{}, translate(err)
	}
	// Do not release a private result if the interactive Session changed while
	// its domain query was in flight. The model-supplied actor is never authority.
	if _, err = t.principal(ctx, in); err != nil {
		return assistant.AgentToolResult{}, err
	}
	raw, err := json.Marshal(value)
	if err != nil || len(raw) > 60<<10 {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	clean, err := redaction.JSON(raw, in.Credential)
	if err != nil || len(clean) > 60<<10 || !bytes.HasPrefix(bytes.TrimSpace(clean), []byte("{")) {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	return assistant.AgentToolResult{OperationID: in.Call.OperationID, State: assistant.AgentToolSucceeded, ResultJSON: clean, ResultSHA256: digest(clean), DomainReceiptID: "read:" + in.Call.OperationID}, nil
}
func translate(err error) error {
	switch {
	case errors.Is(err, store.ErrInvalid), errors.Is(err, bounties.ErrInvalid):
		return access.ErrInvalid
	case errors.Is(err, store.ErrUnauthorized):
		return access.ErrUnauthorized
	case errors.Is(err, store.ErrForbidden), errors.Is(err, bounties.ErrForbidden):
		return access.ErrForbidden
	case errors.Is(err, store.ErrNotFound), errors.Is(err, bounties.ErrNotFound):
		return access.ErrNotFound
	case errors.Is(err, store.ErrConflict), errors.Is(err, bounties.ErrConflict):
		return access.ErrConflict
	case errors.Is(err, store.ErrUnavailable), errors.Is(err, bounties.ErrUnavailable):
		return access.ErrUnavailable
	case errors.Is(err, access.ErrInvalid), errors.Is(err, access.ErrUnauthorized), errors.Is(err, access.ErrForbidden), errors.Is(err, access.ErrNotFound), errors.Is(err, access.ErrConflict), errors.Is(err, access.ErrUnavailable), errors.Is(err, access.ErrLimit):
		return err
	default:
		return access.ErrUnavailable
	}
}

// identifier preserves native string IDs and old positive integer project IDs.
// It never converts through float64 or uses a caller ID as a user identity.
type identifier string

func (v *identifier) UnmarshalJSON(raw []byte) error {
	var s string
	if len(raw) > 0 && raw[0] == '"' {
		if json.Unmarshal(raw, &s) != nil || s != "" && !access.ID(s) {
			return access.ErrInvalid
		}
	} else {
		n, err := strconv.ParseInt(string(raw), 10, 64)
		if err != nil || n < 0 || n > 9007199254740991 {
			return access.ErrInvalid
		}
		s = strconv.FormatInt(n, 10)
	}
	*v = identifier(s)
	return nil
}
