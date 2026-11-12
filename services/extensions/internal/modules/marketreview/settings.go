// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package marketreview

import "context"

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
	if err := e.authorize(ctx, credential, true); err != nil {
		return Settings{}, err
	}
	m, err := DecodeModes(raw)
	if err != nil {
		return Settings{}, err
	}
	r := e.route(ctx)
	if (m.Tool != Off || m.Store != Off) && !r.Valid() {
		return Settings{}, ErrUnavailable
	}
	if err = e.Policy.Save(ctx, m); err != nil {
		return Settings{}, err
	}
	return Settings{m, r}, nil
}
