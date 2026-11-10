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
	return fetchScriptRepositoryWithToken(ctx, client, settings, "")
}

func fetchScriptRepositoryWithToken(ctx context.Context, client *http.Client, settings ScriptRepositorySettings, token string) (*Scripts, error) {
	settings, err := validateScriptRepository(settings)
	if err != nil {
		return nil, err
	}
	if settings.GitHubKeySet && token == "" {
		return nil, ErrUnavailable
	}
	name := strings.TrimPrefix(settings.URL, "https://github.com/")
	archiveURL := "https://codeload.github.com/" + name + "/zip/refs/heads/" + url.PathEscape(settings.Branch)
	ctx, cancel := context.WithTimeout(ctx, 12*time.Second)
	defer cancel()
	if token != "" {
		if !validRepositoryToken(token) {
			return nil, ErrInvalid
		}
		archiveURL = "https://api.github.com/repos/" + name + "/zipball/" + url.PathEscape(settings.Branch)
	}
	// Never let net/http forward a bearer token, cookies or signed download URLs
	// through implicit redirects. Each allowed hop is constructed independently.
	bounded := *client
	bounded.Jar = nil
	bounded.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, archiveURL, nil)
	if err != nil {
		return nil, ErrInvalid
	}
	req.Header.Set("Accept", "application/zip")
	req.Header.Set("User-Agent", "LMM API script repository")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("Accept", "application/vnd.github+json")
		req.Header.Set("X-GitHub-Api-Version", "2022-11-28")
	}
	response, err := bounded.Do(req)
	if err != nil {
		return nil, ErrRepositoryFetch
	}
	if token != "" && (response.StatusCode == http.StatusFound || response.StatusCode == http.StatusTemporaryRedirect) {
		location, parseErr := url.Parse(response.Header.Get("Location"))
		response.Body.Close()
		if parseErr != nil || !repositoryDownloadURL(location, name) || repositoryRedirectContainsCredential(location, token) {
			return nil, ErrRepositoryFetch
		}
		download, err := http.NewRequestWithContext(ctx, http.MethodGet, location.String(), nil)
		if err != nil {
			return nil, ErrRepositoryFetch
		}
		download.Header.Set("Accept", "application/zip")
		download.Header.Set("User-Agent", "LMM API script repository")
		response, err = bounded.Do(download)
		if err != nil {
			return nil, ErrRepositoryFetch
		}
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

func repositoryDownloadURL(u *url.URL, name string) bool {
	return u != nil && u.Scheme == "https" && u.Host == "codeload.github.com" && u.User == nil && u.Opaque == "" && u.Fragment == "" && len(u.String()) <= 4096 && path.Clean(u.Path) == u.Path && strings.HasPrefix(strings.ToLower(u.Path), "/"+strings.ToLower(name)+"/")
}

func repositoryRedirectContainsCredential(u *url.URL, token string) bool {
	decoded, err := url.QueryUnescape(u.RawQuery)
	return err != nil || strings.Contains(u.String(), token) || strings.Contains(u.Path, token) || strings.Contains(decoded, token)
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
