// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package assistant

import (
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"strings"
	"testing"
)

func TestModelRouteAuthorityCASAndRuntimeRead(t *testing.T) {
	s, store, identity := setup(t, Options{})
	repo := &presetHistoryFixture{Store: store}
	history := func() *HistoryService {
		h, err := NewHistory(repo, s.auth, []byte(strings.Repeat("r", 32)))
		if err != nil {
			t.Fatal(err)
		}
		return h
	}
	h := history()
	if group, model, err := h.SelectedRoute(ctx); err != nil || group != "" || model != "" {
		t.Fatal("legacy route", group, model, err)
	}
	req := ModelRoute{Group: "team", Model: "vendor/模型"}
	if _, err := h.RouteSettings(ctx, user); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("ordinary user read", err)
	}
	identity.Set(user, 1, 5)
	if _, err := h.SaveRouteSettings(ctx, user, req); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("read-only admin", err)
	}
	if saved, err := h.SaveRouteSettings(ctx, other, req); err != nil || saved.Revision != 1 {
		t.Fatal("save", saved, err)
	}
	if _, err := h.SaveRouteSettings(ctx, other, req); !errors.Is(err, access.ErrConflict) {
		t.Fatal("stale update", err)
	}
	h = history()
	if group, model, err := h.SelectedRoute(ctx); err != nil || group != req.Group || model != req.Model {
		t.Fatal("persisted runtime preference", group, model, err)
	}
	if saved, err := h.RouteSettings(ctx, user); err != nil || saved.Revision != 1 {
		t.Fatal("admin read", saved, err)
	}
}
