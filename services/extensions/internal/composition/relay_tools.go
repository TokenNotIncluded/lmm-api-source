// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package composition

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/ssmcprelay"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
	"net/http"
	"strconv"
)

type SSMCPRelayConfig struct {
	BaseURL    string `toml:"base_url"`
	TokenFile  string `toml:"token_file"`
	SellerKind string `toml:"seller_kind"`
	SellerID   int64  `toml:"seller_id"`
}

func (r *Runtime) composeRelay(cfg SSMCPRelayConfig, repo store.Repository, authority store.Authority, funds store.Funds, other ...[]byte) error {
	if r.Builtins == nil {
		return errors.New("ssmcpd requires tool market builtins")
	}
	f, ok := funds.(ssmcprelay.Funds)
	if !ok || !f.Available() {
		return errors.New("ssmcpd requires store commerce funding")
	}
	token, err := coreclient.ReadTokenFile(cfg.TokenFile)
	if err != nil {
		return errors.New("ssmcpd service credential unavailable")
	}
	if err := independentRuntimeCredentials(append(other, token)...); err != nil {
		return err
	}
	client, err := ssmcprelay.NewClient(cfg.BaseURL, string(token))
	if err != nil {
		return err
	}
	r.SSMCPRelay, err = ssmcprelay.New(client, repo, f, store.Account{Kind: cfg.SellerKind, ID: cfg.SellerID})
	if err != nil {
		return err
	}
	return r.SSMCPRelay.WithRefundAuthority(authority)
}

type relayTools struct{ service *ssmcprelay.Service }

func (r relayTools) Name() string          { return "ssmcpd" }
func (r relayTools) Handler() http.Handler { return http.NotFoundHandler() }
func (r relayTools) Invoke(ctx context.Context, in toolmarket.BuiltinInvocation, key string) (any, error) {
	user, err := strconv.ParseInt(in.Owner.User, 10, 64)
	if err != nil || user <= 0 {
		return nil, toolmarket.ErrDenied
	}
	switch in.ToolName {
	case "ssmcpd.quota":
		return r.service.Quota(ctx, user)
	case "ssmcpd.purchase":
		var input struct {
			GiB int64 `json:"gib"`
		}
		if json.Unmarshal(in.Arguments, &input) != nil {
			return nil, toolmarket.ErrInvalid
		}
		order, err := r.service.Preview(ctx, user, key, input.GiB)
		if err != nil {
			return nil, err
		}
		return relayPreview(order), nil
	}
	return nil, toolmarket.ErrMissing
}
func (r relayTools) Observe(ctx context.Context, in toolmarket.BuiltinInvocation, key string) (any, error) {
	if in.ToolName == "ssmcpd.quota" {
		return r.Invoke(ctx, in, key)
	}
	user, err := strconv.ParseInt(in.Owner.User, 10, 64)
	if err != nil || user <= 0 || in.ToolName != "ssmcpd.purchase" {
		return nil, toolmarket.ErrDenied
	}
	order, err := r.service.GetOrder(ctx, user, store.ID("ssmcp-relay-order-v1", user, key))
	if err != nil {
		return nil, err
	}
	return relayPreview(order), nil
}
func relayPreview(order ssmcprelay.Order) map[string]any {
	return map[string]any{"order": order, "confirmation_path": "/tool-market?relay_order=" + order.ID}
}
func (r *Runtime) RegisterMarketProviders(enabled func(string) bool) error {
	if r.SSMCPRelay == nil {
		return nil
	}
	return r.Builtins.RegisterPluginTools("ssmcpd", relayTools{r.SSMCPRelay}, func() bool { return enabled("ssmcpd") && enabled("toolmarket") })
}
