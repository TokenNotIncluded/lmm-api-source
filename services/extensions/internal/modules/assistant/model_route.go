// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package assistant

import (
	"context"
	"errors"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/storage"
)

type ModelRoute struct {
	Group    string `json:"group"`
	Model    string `json:"model"`
	Revision int64  `json:"revision"`
}

func routeLabel(value string, limit int) bool {
	return value != "" && len(value) <= limit && utf8.ValidString(value) &&
		strings.IndexFunc(value, func(r rune) bool { return unicode.IsSpace(r) || unicode.IsControl(r) }) == -1
}

func (s *HistoryService) modelRoute(ctx context.Context, update *ModelRoute) (ModelRoute, error) {
	repo, ok := s.repo.(storage.Repository)
	if !ok {
		return ModelRoute{}, access.ErrUnavailable
	}
	var route ModelRoute
	err := repo.Within(ctx, "plugin:model-route", func(tx storage.Tx) error {
		err := tx.Get("preferences", "model-route", &route)
		if err != nil && !errors.Is(err, access.ErrNotFound) {
			return err
		}
		if err == nil && (!routeLabel(route.Group, 64) || !routeLabel(route.Model, 256) || route.Revision <= 0 || route.Revision > maxSafeHistoryID) {
			return access.ErrUnavailable
		}
		if update == nil {
			return nil
		}
		if route.Revision != update.Revision {
			return access.ErrConflict
		}
		if route.Group == update.Group && route.Model == update.Model {
			return nil
		}
		route = *update
		route.Revision++
		return tx.Put("preferences", "model-route", route)
	})
	return route, err
}

// SelectedRoute supplies the plugin choice to the existing authenticated Core
// RPC. A missing row preserves the deployment's published legacy binding.
func (s *HistoryService) SelectedRoute(ctx context.Context) (string, string, error) {
	route, err := s.modelRoute(ctx, nil)
	return route.Group, route.Model, err
}

func (s *HistoryService) RouteSettings(ctx context.Context, token string) (ModelRoute, error) {
	actor, err := s.actor(ctx, token)
	if err != nil {
		return ModelRoute{}, err
	}
	if actor.Level < 5 {
		return ModelRoute{}, access.ErrForbidden
	}
	return s.modelRoute(ctx, nil)
}

func (s *HistoryService) SaveRouteSettings(ctx context.Context, token string, route ModelRoute) (ModelRoute, error) {
	actor, err := s.actor(ctx, token)
	if err != nil {
		return ModelRoute{}, err
	}
	if actor.Level != 6 {
		return ModelRoute{}, access.ErrForbidden
	}
	if !routeLabel(route.Group, 64) || !routeLabel(route.Model, 256) || route.Revision < 0 || route.Revision >= maxSafeHistoryID {
		return ModelRoute{}, access.ErrInvalid
	}
	return s.modelRoute(ctx, &route)
}
