// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"archive/zip"
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"net/url"
	"path"
	"sort"
	"strings"
	"time"
)

const maxRepositoryArchiveBytes = 16 << 20

var ErrRepositoryFetch = errors.New("script repository fetch failed")

// Fetch a single archive, so every imported file belongs to one Git snapshot.
// The RSS client supplies DNS pinning, redirect checks, no proxy and a timeout.
// No git subprocess, filesystem extraction, script execution or caller URL fetch.
func fetchScriptRepository(ctx context.Context, client *http.Client, settings ScriptRepositorySettings) (*Scripts, error) {
	settings, err := validateScriptRepository(settings)
	if err != nil {
		return nil, err
	}
	name := strings.TrimPrefix(settings.URL, "https://github.com/")
	archiveURL := "https://codeload.github.com/" + name + "/zip/refs/heads/" + url.PathEscape(settings.Branch)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, archiveURL, nil)
	if err != nil {
		return nil, ErrInvalid
	}
	req.Header.Set("Accept", "application/zip")
	req.Header.Set("User-Agent", "LMM API script repository")
	response, err := client.Do(req)
	if err != nil {
		return nil, ErrRepositoryFetch
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, ErrRepositoryFetch
	}
	if response.ContentLength > maxRepositoryArchiveBytes {
		return nil, ErrInvalid
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, maxRepositoryArchiveBytes+1))
	if err != nil {
		return nil, ErrRepositoryFetch
	}
	if len(raw) > maxRepositoryArchiveBytes {
		return nil, ErrInvalid
	}
	return readScriptArchive(raw)
}

func readScriptArchive(raw []byte) (*Scripts, error) {
	archive, err := zip.NewReader(bytes.NewReader(raw), int64(len(raw)))
	if err != nil || len(archive.File) > 4096 {
		return nil, ErrInvalid
	}
	files := map[string][]byte{}
	root := ""
	total := 0
	for _, entry := range archive.File {
		name := strings.TrimSuffix(entry.Name, "/")
		if name == "" || path.Clean(name) != name || strings.Contains(name, "\\") || strings.HasPrefix(name, "/") || name == ".." || strings.HasPrefix(name, "../") {
			return nil, ErrInvalid
		}
		parts := strings.Split(name, "/")
		if root == "" {
			root = parts[0]
		}
		if parts[0] != root {
			return nil, ErrInvalid
		}
		if entry.FileInfo().IsDir() {
			continue
		}
		// The existing catalogue serves root-level script basenames only.
		if len(parts) != 2 || !ValidScriptName(parts[1]) {
			continue
		}
		if !entry.Mode().IsRegular() || entry.UncompressedSize64 > MaxScriptBytes || len(files) >= MaxScripts {
			return nil, ErrInvalid
		}
		if _, duplicate := files[parts[1]]; duplicate {
			return nil, ErrInvalid
		}
		reader, err := entry.Open()
		if err != nil {
			return nil, ErrInvalid
		}
		data, readErr := io.ReadAll(io.LimitReader(reader, MaxScriptBytes+1))
		closeErr := reader.Close()
		if readErr != nil || closeErr != nil || len(data) > MaxScriptBytes {
			return nil, ErrInvalid
		}
		total += len(data)
		if total > MaxSnapshotBytes {
			return nil, ErrInvalid
		}
		files[parts[1]] = data
	}
	return repositoryScripts(files, time.Now().UTC())
}

func repositoryScripts(files map[string][]byte, updated time.Time) (*Scripts, error) {
	// An empty/invalid archive must not silently delete a working catalogue.
	if len(files) == 0 || len(files) > MaxScripts {
		return nil, ErrInvalid
	}
	snapshot := &Scripts{entries: map[string]scriptSnapshot{}, catalogue: []ScriptMeta{}}
	total := 0
	for name, data := range files {
		total += len(data)
		if !ValidScriptName(name) || len(data) > MaxScriptBytes || total > MaxSnapshotBytes {
			return nil, ErrInvalid
		}
		meta := ScriptMeta{Name: name, Size: len(data), Updated: updated.UTC().Format(time.RFC3339Nano)}
		snapshot.entries[name] = scriptSnapshot{meta: meta, data: data}
		snapshot.catalogue = append(snapshot.catalogue, meta)
	}
	sort.Slice(snapshot.catalogue, func(i, j int) bool { return snapshot.catalogue[i].Name < snapshot.catalogue[j].Name })
	return snapshot, nil
}
