// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package assistant

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"regexp"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/storage"
)

const (
	MaxPromptPresetConfiguration = 48000
	promptPresetChunkRunes       = 2400
	promptPresetScope            = "public:prompt-presets"
	promptPresetRecord           = "pre-conversation-presets"
)

var promptPresetID = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)
var promptPresetLocale = regexp.MustCompile(`^[A-Za-z0-9_-]{1,32}$`)

type PromptPresetSettings struct {
	Configuration string `json:"configuration"`
	Revision      int64  `json:"revision"`
	UpdatedAt     int64  `json:"updated_at"`
}

type promptPresetVersion struct {
	Revision  int64 `json:"revision"`
	UpdatedAt int64 `json:"updated_at"`
	Chunks    int   `json:"chunks"`
}

type promptPresetChunk struct {
	Content *string `json:"content"`
}

type configuredPromptPreset struct {
	id     string
	label  map[string]string
	prompt map[string]string
}

type PromptPreset struct {
	ID     string `json:"id"`
	Prompt string `json:"prompt"`
	Label  string `json:"label"`
	Source string `json:"source"`
}
type PromptPresetSet struct {
	Generation int64          `json:"generation"`
	Version    string         `json:"version"`
	Presets    []PromptPreset `json:"presets"`
}

// IsBuiltinPromptPreset recognizes only the reviewed starter catalog. A known
// ID does not authorize model behavior, tools, funding or reward claims.
func IsBuiltinPromptPreset(id string) bool {
	_, ok := requiredPromptPresetCopy[id]
	return ok
}

// These are main's reviewed fallback seeds, not generated/custom presets or
// evidence that gift, discounts or access-review business is already available.
func HistoryPresets(language string) PromptPresetSet {
	locale := promptPresetLanguage(language)
	if _, ok := requiredPromptPresetCopy["getting_started"][locale]; !ok {
		locale = "en"
	}
	set := PromptPresetSet{Generation: 0, Version: "backend-seed-v3", Presets: []PromptPreset{}}
	for _, id := range []string{"ai_recommendation", "getting_started", "new_user_gift", "weekly_discount"} {
		copy := requiredPromptPresetCopy[id][locale]
		set.Presets = append(set.Presets, PromptPreset{id, copy, copy, "default"})
	}
	return set
}

func promptPresetLanguage(language string) string {
	language = strings.ToLower(strings.ReplaceAll(strings.TrimSpace(language), "_", "-"))
	switch {
	case language == "zhtw" || language == "zh-tw" || language == "zh-hk" || language == "zh-mo" || strings.HasPrefix(language, "zh-hant"):
		return "zh-TW"
	case language == "zhcn" || language == "zh" || strings.HasPrefix(language, "zh-"):
		return "zh"
	default:
		for _, supported := range []string{"en", "fr", "ja", "ru", "vi"} {
			if language == supported || strings.HasPrefix(language, supported+"-") {
				return supported
			}
		}
	}
	return language
}

// Decode the small administrator-authored grammar directly: ordinary JSON
// unmarshalling would silently accept duplicate fields, nulls and case aliases.
func parsePromptPresets(raw string) ([]configuredPromptPreset, error) {
	if !utf8.ValidString(raw) || utf8.RuneCountInString(raw) > MaxPromptPresetConfiguration {
		return nil, access.ErrInvalid
	}
	if strings.TrimSpace(raw) == "" {
		return nil, nil
	}
	d := json.NewDecoder(strings.NewReader(raw))
	first, err := d.Token()
	if err != nil || first != json.Delim('[') {
		return nil, access.ErrInvalid
	}
	entries := []configuredPromptPreset{}
	seen := map[string]bool{}
	for d.More() {
		if len(entries) >= 20 {
			return nil, access.ErrInvalid
		}
		entry, err := decodePromptPreset(d)
		if err != nil || seen[entry.id] {
			return nil, access.ErrInvalid
		}
		seen[entry.id] = true
		entries = append(entries, entry)
	}
	if last, err := d.Token(); err != nil || last != json.Delim(']') {
		return nil, access.ErrInvalid
	}
	if _, err := d.Token(); err != io.EOF {
		return nil, access.ErrInvalid
	}
	return entries, nil
}

func decodePromptPreset(d *json.Decoder) (configuredPromptPreset, error) {
	var entry configuredPromptPreset
	first, err := d.Token()
	if err != nil || first != json.Delim('{') {
		return entry, access.ErrInvalid
	}
	seen := map[string]bool{}
	for d.More() {
		token, err := d.Token()
		key, ok := token.(string)
		if err != nil || !ok || seen[key] {
			return entry, access.ErrInvalid
		}
		seen[key] = true
		switch key {
		case "id":
			token, err = d.Token()
			entry.id, ok = token.(string)
			if err != nil || !ok || !promptPresetID.MatchString(entry.id) {
				return entry, access.ErrInvalid
			}
		case "label":
			entry.label, err = decodePromptPresetCopy(d, 80)
		case "prompt":
			entry.prompt, err = decodePromptPresetCopy(d, 2000)
		default:
			return entry, access.ErrInvalid
		}
		if err != nil {
			return entry, err
		}
	}
	if last, err := d.Token(); err != nil || last != json.Delim('}') || len(seen) != 3 {
		return entry, access.ErrInvalid
	}
	return entry, nil
}

func decodePromptPresetCopy(d *json.Decoder, maxRunes int) (map[string]string, error) {
	valid := func(value string) bool {
		length := utf8.RuneCountInString(value)
		return length > 0 && length <= maxRunes && !strings.ContainsRune(value, 0)
	}
	first, err := d.Token()
	if err != nil {
		return nil, access.ErrInvalid
	}
	if value, ok := first.(string); ok {
		value = strings.TrimSpace(value)
		if !valid(value) {
			return nil, access.ErrInvalid
		}
		return map[string]string{"default": value}, nil
	}
	if first != json.Delim('{') {
		return nil, access.ErrInvalid
	}
	values := map[string]string{}
	for d.More() {
		token, err := d.Token()
		locale, ok := token.(string)
		if err != nil || !ok || !promptPresetLocale.MatchString(locale) || len(values) >= 16 {
			return nil, access.ErrInvalid
		}
		locale = promptPresetLanguage(locale)
		if _, exists := values[locale]; exists {
			return nil, access.ErrInvalid
		}
		token, err = d.Token()
		value, ok := token.(string)
		value = strings.TrimSpace(value)
		if err != nil || !ok || !valid(value) {
			return nil, access.ErrInvalid
		}
		values[locale] = value
	}
	if last, err := d.Token(); err != nil || last != json.Delim('}') || values["default"] == "" {
		return nil, access.ErrInvalid
	}
	return values, nil
}

// Configuration is split across twenty fixed slots. Even worst-case JSON
// escaping of 2,400 runes stays below the existing 16 KiB record limit. Slots
// are reused, so arbitrary preset IDs/locales never create unbounded records.
func readPromptPresetSettings(tx storage.Tx) (PromptPresetSettings, []configuredPromptPreset, error) {
	var version promptPresetVersion
	if err := tx.Get("preferences", promptPresetRecord, &version); errors.Is(err, access.ErrNotFound) {
		return PromptPresetSettings{}, nil, nil
	} else if err != nil {
		return PromptPresetSettings{}, nil, err
	}
	if version.Revision < 1 || version.Revision > maxSafeHistoryID || version.UpdatedAt < 1 || version.UpdatedAt > maxSafeHistoryID || version.Chunks < 0 || version.Chunks > 20 {
		return PromptPresetSettings{}, nil, access.ErrUnavailable
	}
	var raw strings.Builder
	for n := 0; n < version.Chunks; n++ {
		var chunk promptPresetChunk
		if err := tx.Get("preferences", promptPresetRecord+":"+strconv.Itoa(n), &chunk); err != nil || chunk.Content == nil || *chunk.Content == "" || !utf8.ValidString(*chunk.Content) || utf8.RuneCountInString(*chunk.Content) > promptPresetChunkRunes {
			return PromptPresetSettings{}, nil, access.ErrUnavailable
		}
		raw.WriteString(*chunk.Content)
	}
	entries, err := parsePromptPresets(raw.String())
	if err != nil {
		return PromptPresetSettings{}, nil, access.ErrUnavailable
	}
	return PromptPresetSettings{raw.String(), version.Revision, version.UpdatedAt}, entries, nil
}

func publishedPromptPresets(settings PromptPresetSettings, entries []configuredPromptPreset, language string) PromptPresetSet {
	if entries == nil {
		set := HistoryPresets(language)
		set.Generation = settings.Revision
		return set
	}
	locale := promptPresetLanguage(language)
	copyFor := func(copies map[string]string) string {
		if value, ok := copies[locale]; ok {
			return value
		}
		return copies["default"]
	}
	set := PromptPresetSet{Generation: settings.Revision, Version: "custom-v1", Presets: []PromptPreset{}}
	for _, entry := range entries {
		set.Presets = append(set.Presets, PromptPreset{entry.id, copyFor(entry.prompt), copyFor(entry.label), "custom"})
	}
	return set
}

func (s *HistoryService) Presets(ctx context.Context, language string) (PromptPresetSet, error) {
	repo, ok := s.repo.(storage.Repository)
	if !ok || access.Missing(repo) {
		return PromptPresetSet{}, access.ErrUnavailable
	}
	var set PromptPresetSet
	err := repo.Within(ctx, promptPresetScope, func(tx storage.Tx) error {
		settings, entries, err := readPromptPresetSettings(tx)
		if err == nil {
			set = publishedPromptPresets(settings, entries, language)
		}
		return err
	})
	return set, err
}

// Preset IDs are UI hints only. Neither these IDs nor stored copy are passed to
// a tool, a funding policy or a system-message constructor.
func (s *HistoryService) CheckPreset(ctx context.Context, id string) error {
	if !promptPresetID.MatchString(id) {
		return access.ErrNotFound
	}
	set, err := s.Presets(ctx, "")
	if err != nil {
		return err
	}
	for _, entry := range set.Presets {
		if entry.ID == id {
			return nil
		}
	}
	return access.ErrNotFound
}

func (s *HistoryService) PresetSettings(ctx context.Context, token string) (PromptPresetSettings, error) {
	p, err := s.actor(ctx, token)
	if err != nil {
		return PromptPresetSettings{}, err
	}
	if p.Level < 5 {
		return PromptPresetSettings{}, access.ErrForbidden
	}
	repo, ok := s.repo.(storage.Repository)
	if !ok || access.Missing(repo) {
		return PromptPresetSettings{}, access.ErrUnavailable
	}
	var settings PromptPresetSettings
	err = repo.Within(ctx, promptPresetScope, func(tx storage.Tx) error {
		var err error
		settings, _, err = readPromptPresetSettings(tx)
		return err
	})
	return settings, err
}

func (s *HistoryService) UpdatePresetSettings(ctx context.Context, token, configuration string, revision int64) (PromptPresetSettings, error) {
	p, err := s.actor(ctx, token)
	if err != nil {
		return PromptPresetSettings{}, err
	}
	if p.Level != 6 {
		return PromptPresetSettings{}, access.ErrForbidden
	}
	if _, err = parsePromptPresets(configuration); err != nil || revision < 0 || revision > maxSafeHistoryID {
		return PromptPresetSettings{}, access.ErrInvalid
	}
	configuration = strings.TrimSpace(configuration)
	repo, ok := s.repo.(storage.Repository)
	if !ok || access.Missing(repo) {
		return PromptPresetSettings{}, access.ErrUnavailable
	}
	var saved PromptPresetSettings
	err = repo.Within(ctx, promptPresetScope, func(tx storage.Tx) error {
		previous, _, err := readPromptPresetSettings(tx)
		if err != nil {
			return err
		}
		if previous.Revision != revision {
			return access.ErrConflict
		}
		if revision == maxSafeHistoryID {
			return access.ErrLimit
		}
		now := s.now().Unix()
		if now < 1 || now > maxSafeHistoryID {
			return access.ErrUnavailable
		}
		runes := []rune(configuration)
		chunks := (len(runes) + promptPresetChunkRunes - 1) / promptPresetChunkRunes
		previousChunks := (utf8.RuneCountInString(previous.Configuration) + promptPresetChunkRunes - 1) / promptPresetChunkRunes
		for n := 0; n < max(chunks, previousChunks); n++ {
			value := ""
			if n < chunks {
				value = string(runes[n*promptPresetChunkRunes : min((n+1)*promptPresetChunkRunes, len(runes))])
			}
			if err := tx.Put("preferences", promptPresetRecord+":"+strconv.Itoa(n), promptPresetChunk{&value}); err != nil {
				return err
			}
		}
		saved = PromptPresetSettings{configuration, revision + 1, now}
		return tx.Put("preferences", promptPresetRecord, promptPresetVersion{saved.Revision, saved.UpdatedAt, chunks})
	})
	return saved, err
}

type promptPresetClicks struct {
	ID       string `json:"id"`
	Revision int64  `json:"revision"`
	Clicks   int64  `json:"clicks"`
}

func clickCustomPromptPreset(tx storage.Tx, settings PromptPresetSettings, slot int, id string) error {
	key := promptPresetRecord + ":clicks:" + strconv.Itoa(slot)
	var counter promptPresetClicks
	if err := tx.Get("preferences", key, &counter); err != nil && !errors.Is(err, access.ErrNotFound) {
		return err
	}
	if counter.Clicks < 0 || counter.Clicks > maxSafeHistoryID || counter.Revision < 0 || counter.Revision > settings.Revision {
		return access.ErrUnavailable
	}
	if counter.Revision != settings.Revision || counter.ID != id {
		counter = promptPresetClicks{ID: id, Revision: settings.Revision}
	}
	if counter.Clicks == maxSafeHistoryID {
		return access.ErrLimit
	}
	counter.Clicks++
	return tx.Put("preferences", key, counter)
}
