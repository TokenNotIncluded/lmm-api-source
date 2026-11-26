// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"errors"
)

// NewPluginDispatcher binds only the market execution authority. The plugin
// receives no drawing, bounty, or wallet service credentials.
func NewPluginDispatcher(ctx context.Context, gate BuiltinGate) (*BuiltinDispatcher, error) {
	d := &BuiltinDispatcher{Gate: gate, pluginsOnly: true}
	if err := d.CheckReadiness(ctx); err != nil {
		return nil, err
	}
	return d, nil
}

// PluginTools implements reviewed catalog entries. Observe reconciles an existing
// operation following an uncertain outcome and must never create another order.
type PluginTools interface {
	Invoke(context.Context, BuiltinInvocation, string) (any, error)
	Observe(context.Context, BuiltinInvocation, string) (any, error)
}
type pluginToolsBinding struct {
	tools   PluginTools
	enabled func() bool
}

// RegisterPluginTools runs during host composition, before serving requests.
// The provider cannot supply new schemas or override a privileged built-in.
func (d *BuiltinDispatcher) RegisterPluginTools(key string, p PluginTools, enabled func() bool) error {
	if d == nil || missingInterface(p) || enabled == nil || key == "wallet" || key == "drawing" || key == "open_source_bounties" {
		return errors.New("invalid market provider")
	}
	for _, def := range CompiledBuiltinDefinitions() {
		if def.Key != key {
			continue
		}
		if d.providers == nil {
			d.providers = map[string]pluginToolsBinding{}
		}
		if _, exists := d.providers[key]; exists {
			return errors.New("duplicate market provider")
		}
		d.providers[key] = pluginToolsBinding{p, enabled}
		return nil
	}
	return errors.New("market provider missing from compiled catalog")
}
