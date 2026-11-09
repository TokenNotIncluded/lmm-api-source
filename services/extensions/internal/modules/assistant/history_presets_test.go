// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/storage"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/testkit"
)

type presetHistoryFixture struct {
	historySpy
	*testkit.Store
}

func TestPromptPresetConfigurationRejectsAmbiguousOrUnboundedInput(t *testing.T) {
	valid := `{"id":"custom","label":"Label","prompt":"Prompt"}`
	for _, raw := range []string{
		`null`, `{}`, `[null]`, `[] {}`, `[[]]`,
		`[{"id":"custom","label":"Label"}]`,
		`[{"id":"custom","ID":"other","label":"Label","prompt":"Prompt"}]`,
		`[{"id":"custom","label":"Label","prompt":"Prompt","prompt":"Changed"}]`,
		`[{"id":"custom","label":"Label","prompt":"Prompt","system":"elevate"}]`,
		`[{"id":"custom","label":null,"prompt":"Prompt"}]`,
		`[{"id":"custom","label":"Label","prompt":{"en":"No default"}}]`,
		`[{"id":"custom","label":"Label","prompt":{"default":"One","default":"Two"}}]`,
		`[{"id":"custom","label":"Label","prompt":{"default":"One","zh-TW":"Two","zh_Hant":"Three"}}]`,
		`[{"id":"custom","label":"Label","prompt":{"default":"One","bad key":"Two"}}]`,
		`[{"id":"custom","label":"Label","prompt":{"default":[]}}]`,
		`[{"id":"custom","label":"Label","prompt":"\u0000"}]`,
		`[{"id":" custom","label":"Label","prompt":"Prompt"}]`,
		`[` + valid + `,` + valid + `]`,
		`[{"id":"` + strings.Repeat("i", 65) + `","label":"Label","prompt":"Prompt"}]`,
		`[{"id":"custom","label":"` + strings.Repeat("中", 81) + `","prompt":"Prompt"}]`,
		`[{"id":"custom","label":"Label","prompt":"` + strings.Repeat("😀", 2001) + `"}]`,
		strings.Repeat(" ", MaxPromptPresetConfiguration+1),
		string([]byte{0xff}),
	} {
		if _, err := parsePromptPresets(raw); !errors.Is(err, access.ErrInvalid) {
			t.Fatalf("invalid configuration accepted: %.100q: %v", raw, err)
		}
	}
	for _, raw := range []string{"", " \n\t", "[]", "[" + valid + "]", `[{"id":"custom","label":"` + strings.Repeat("中", 80) + `","prompt":"` + strings.Repeat("😀", 2000) + `"}]`} {
		if _, err := parsePromptPresets(raw); err != nil {
			t.Fatalf("valid configuration rejected: %v", err)
		}
	}
	entries := []string{}
	for n := 0; n < 21; n++ {
		entries = append(entries, fmt.Sprintf(`{"id":"p%d","label":"Label","prompt":"Prompt"}`, n))
	}
	if _, err := parsePromptPresets("[" + strings.Join(entries[:20], ",") + "]"); err != nil {
		t.Fatal("twenty presets rejected", err)
	}
	if _, err := parsePromptPresets("[" + strings.Join(entries, ",") + "]"); !errors.Is(err, access.ErrInvalid) {
		t.Fatal("more than twenty presets accepted", err)
	}
}

func TestPromptPresetSettingsAuthorityCASLocalesAndMissingStorage(t *testing.T) {
	s, store, identity := setup(t, Options{})
	repo := &presetHistoryFixture{Store: store}
	history, err := NewHistory(repo, s.auth, []byte(strings.Repeat("c", 32)))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := history.PresetSettings(ctx, user); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("L1 read", err)
	}
	identity.Set(user, 1, 5)
	initial, err := history.PresetSettings(ctx, user)
	if err != nil || initial != (PromptPresetSettings{}) {
		t.Fatal(initial, err)
	}
	if _, err := history.UpdatePresetSettings(ctx, user, "[]", 0); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("L5 write", err)
	}
	raw := `[{"id":"custom","label":"Default label","prompt":{"default":"Default prompt","zh_Hant":"繁體提示","fr-FR":"Texte français"}}]`
	saved, err := history.UpdatePresetSettings(ctx, other, raw, 0)
	if err != nil || saved.Configuration != raw || saved.Revision != 1 || saved.UpdatedAt <= 0 {
		t.Fatal(saved, err)
	}
	for language, prompt := range map[string]string{"zh-TW": "繁體提示", "fr": "Texte français", "ja": "Default prompt"} {
		set, err := history.Presets(ctx, language)
		if err != nil || set.Generation != 1 || set.Version != "custom-v1" || len(set.Presets) != 1 || set.Presets[0] != (PromptPreset{"custom", prompt, "Default label", "custom"}) {
			t.Fatal(language, set, err)
		}
	}
	if err := history.CheckPreset(ctx, "custom"); err != nil {
		t.Fatal(err)
	}
	if err := history.CheckPreset(ctx, "getting_started"); !errors.Is(err, access.ErrNotFound) {
		t.Fatal("unpublished builtin accepted", err)
	}
	if _, err := history.UpdatePresetSettings(ctx, other, "[]", 0); !errors.Is(err, access.ErrConflict) {
		t.Fatal("stale revision accepted", err)
	}
	if _, err := history.UpdatePresetSettings(ctx, other, "[]", 1); err != nil {
		t.Fatal(err)
	}
	hidden, err := history.Presets(ctx, "en")
	if err != nil || hidden.Version != "custom-v1" || hidden.Presets == nil || len(hidden.Presets) != 0 {
		t.Fatal(hidden, err)
	}
	if _, err := history.UpdatePresetSettings(ctx, other, " \t ", 2); err != nil {
		t.Fatal(err)
	}
	defaults, err := history.Presets(ctx, "zh")
	if err != nil || defaults.Generation != 3 || defaults.Version != "backend-seed-v3" || len(defaults.Presets) != 4 {
		t.Fatal(defaults, err)
	}
	identity.Set(other, 2, 5)
	if _, err := history.UpdatePresetSettings(ctx, other, raw, 3); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("demotion not rechecked", err)
	}
	identity.Revoke(user)
	if _, err := history.PresetSettings(ctx, user); !errors.Is(err, access.ErrUnauthorized) {
		t.Fatal("revocation not rechecked", err)
	}
	store.Fail = true
	if _, err := history.Presets(ctx, "en"); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("storage failure silently returned defaults", err)
	}
	missing, err := NewHistory(&historySpy{}, s.auth, []byte(strings.Repeat("c", 32)))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := missing.Presets(ctx, "en"); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("missing repository silently returned defaults", err)
	}
}

func TestPromptPresetChunksStayWithinStorageLimitAndDoNotRetainDeletedCopy(t *testing.T) {
	s, store, _ := setup(t, Options{})
	history, err := NewHistory(&presetHistoryFixture{Store: store}, s.auth, []byte(strings.Repeat("c", 32)))
	if err != nil {
		t.Fatal(err)
	}
	entries := []string{}
	for n := 0; n < 20; n++ {
		entries = append(entries, fmt.Sprintf(`{"id":"p%d","label":"<>&","prompt":"%s"}`, n, strings.Repeat("<>&😀", 500)))
	}
	raw := "[" + strings.Join(entries, ",") + "]"
	saved, err := history.UpdatePresetSettings(ctx, other, raw, 0)
	if err != nil {
		t.Fatal(err)
	}
	got, err := history.PresetSettings(ctx, other)
	if err != nil || got != saved {
		t.Fatal("lossy chunk roundtrip", err)
	}
	if len(store.Data[promptPresetScope]) > 21 {
		t.Fatal("unbounded configuration records")
	}
	for _, record := range store.Data[promptPresetScope] {
		if len(record) > 16384 {
			t.Fatal("record exceeds production storage limit", len(record))
		}
	}
	if _, err = history.UpdatePresetSettings(ctx, other, "", saved.Revision); err != nil {
		t.Fatal(err)
	}
	for key, record := range store.Data[promptPresetScope] {
		if key == "preferences/"+promptPresetRecord {
			continue
		}
		var chunk promptPresetChunk
		if json.Unmarshal(record, &chunk) != nil || chunk.Content == nil || *chunk.Content != "" {
			t.Fatal("deleted copy retained in old slot")
		}
	}
	if err := store.Within(ctx, promptPresetScope, func(tx storage.Tx) error {
		return tx.Put("preferences", promptPresetRecord, promptPresetVersion{Revision: 4, UpdatedAt: 1, Chunks: 21})
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := history.Presets(ctx, "en"); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("corrupt config silently became defaults", err)
	}
}
