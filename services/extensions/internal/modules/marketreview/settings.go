// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package marketreview

import (
	"context"
	"encoding/json"
)

type Administrator interface {
	Authorize(context.Context, string, bool) (int64, error)
}
type Settings struct {
	Modes
	Route
}

func (e *Engine) authorize(ctx context.Context, credential string, root bool) error {
	if e == nil || e.Policy == nil || e.Classifier == nil {
		return ErrUnavailable
	}
	a, ok := e.Classifier.(Administrator)
	if !ok {
		return ErrUnavailable
	}
	_, err := a.Authorize(ctx, credential, root)
	return err
}
func (e *Engine) route(ctx context.Context) Route {
	if e == nil || e.Classifier == nil {
		return Route{Engine: "openai_moderation", SupportedInputs: []string{"text"}, Categories: []string{}}
	}
	r, err := e.Classifier.Route(ctx)
	if err != nil {
		return Route{Engine: "openai_moderation", SupportedInputs: []string{"text"}, Categories: []string{}}
	}
	return r
}
func (e *Engine) Route(ctx context.Context) Route { return e.route(ctx) }
func (e *Engine) Settings(ctx context.Context, credential string) (Settings, error) {
	if err := e.authorize(ctx, credential, false); err != nil {
		return Settings{}, err
	}
	out := Settings{Route: e.route(ctx)}
	err := e.With(ctx, func(m Modes) error { out.Modes = m; return nil })
	return out, err
}
func (e *Engine) SaveSettings(ctx context.Context, credential string, raw []byte) (Settings, error) {
	return e.saveSettings(ctx, credential, raw, DecodeModes)
}

func (e *Engine) PatchSettings(ctx context.Context, credential string, raw []byte) (Settings, error) {
	return e.saveSettings(ctx, credential, raw, decodeModePatch)
}

func decodeModePatch(raw []byte) (Modes, error) {
	var fields map[string]Mode
	if err := json.Unmarshal(raw, &fields); err != nil || len(fields) == 0 {
		return Modes{}, ErrInvalid
	}
	var changes Modes
	for field, mode := range fields {
		if !validMode(mode) {
			return Modes{}, ErrInvalid
		}
		switch field {
		case "tool_mode":
			changes.Tool = mode
		case "store_mode":
			changes.Store = mode
		default:
			return Modes{}, ErrInvalid
		}
	}
	return changes, nil
}

func (e *Engine) saveSettings(ctx context.Context, credential string, raw []byte, decode func([]byte) (Modes, error)) (Settings, error) {
	if err := e.authorize(ctx, credential, true); err != nil {
		return Settings{}, err
	}
	changes, err := decode(raw)
	if err != nil {
		return Settings{}, err
	}
	out := Settings{Route: e.route(ctx)}
	err = e.Policy.Update(ctx, func(current Modes) (Modes, error) {
		if changes.Tool != "" {
			current.Tool = changes.Tool
		}
		if changes.Store != "" {
			current.Store = changes.Store
		}
		if (current.Tool != Off || current.Store != Off) && !out.Route.Valid() {
			return Modes{}, ErrUnavailable
		}
		out.Modes = current
		return current, nil
	})
	if err != nil {
		return Settings{}, err
	}
	return out, nil
}
