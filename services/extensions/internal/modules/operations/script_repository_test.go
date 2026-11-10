// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"archive/zip"
	"bytes"
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync/atomic"
	"testing"
)

func scriptArchive(t *testing.T, files map[string]string) []byte {
	t.Helper()
	var raw bytes.Buffer
	w := zip.NewWriter(&raw)
	for name, data := range files {
		file, err := w.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err = file.Write([]byte(data)); err != nil {
			t.Fatal(err)
		}
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	return raw.Bytes()
}

func TestScriptRepositoryValidationAndArchive(t *testing.T) {
	valid := ScriptRepositorySettings{URL: "https://github.com/TokenNotIncluded/lmm-scripts.git", Branch: "release/v1", Revision: 2}
	if got, err := validateScriptRepository(valid); err != nil || got.URL != "https://github.com/TokenNotIncluded/lmm-scripts" {
		t.Fatal(got, err)
	}
	for _, raw := range []string{"http://github.com/a/b", "https://github.com.evil/a/b", "https://user:secret@github.com/a/b", "https://127.0.0.1/a/b", "https://github.com/a/b/tree/main", "https://github.com/a/b?token=secret", "https://github.com/a/b#fragment", "file:///a/b"} {
		settings := valid
		settings.URL = raw
		if _, err := validateScriptRepository(settings); !errors.Is(err, ErrInvalid) {
			t.Fatal(raw, err)
		}
	}
	for _, branch := range []string{"", "../main", "a//b", "a/.hidden", "a.lock", "-main", "main?key=secret", "a\\b"} {
		settings := valid
		settings.Branch = branch
		if _, err := validateScriptRepository(settings); !errors.Is(err, ErrInvalid) {
			t.Fatal(branch, err)
		}
	}
	archive := scriptArchive(t, map[string]string{"repo-main/a.sh": "#!/bin/sh\r\necho A\n", "repo-main/b.ps1": "Write-Host B", "repo-main/README.md": "docs", "repo-main/nested/hidden.sh": "hidden"})
	scripts, err := readScriptArchive(archive)
	if err != nil {
		t.Fatal(err)
	}
	list, _ := scripts.List()
	if len(list) != 2 || list[0].Name != "a.sh" || list[1].Name != "b.ps1" {
		t.Fatal(list)
	}
	if raw, err := scripts.Raw("a.sh"); err != nil || string(raw) != "#!/bin/sh\r\necho A\n" {
		t.Fatal(string(raw), err)
	}
	for _, files := range []map[string]string{
		{"repo-main/README.md": "empty"}, {"repo-main/../a.sh": "escape"},
		{"repo-main/a.sh": "ok", "other/b.sh": "mixed"},
		{"repo-main/a.sh": strings.Repeat("a", MaxScriptBytes+1)},
	} {
		if _, err := readScriptArchive(scriptArchive(t, files)); !errors.Is(err, ErrInvalid) {
			t.Fatal(files, err)
		}
	}
	if _, err := readScriptArchive([]byte("not a zip")); !errors.Is(err, ErrInvalid) {
		t.Fatal(err)
	}
	var duplicate bytes.Buffer
	w := zip.NewWriter(&duplicate)
	for i := 0; i < 2; i++ {
		f, _ := w.Create("root/a.sh")
		fmt.Fprint(f, "duplicate")
	}
	w.Close()
	if _, err := readScriptArchive(duplicate.Bytes()); !errors.Is(err, ErrInvalid) {
		t.Fatal("duplicate", err)
	}
	var symlink bytes.Buffer
	w = zip.NewWriter(&symlink)
	header := &zip.FileHeader{Name: "root/a.sh"}
	header.SetMode(0o777 | os.ModeSymlink)
	f, _ := w.CreateHeader(header)
	fmt.Fprint(f, "/etc/passwd")
	w.Close()
	if _, err := readScriptArchive(symlink.Bytes()); !errors.Is(err, ErrInvalid) {
		t.Fatal("symlink", err)
	}
}

func TestScriptRepositoryRealHTTP(t *testing.T) {
	archive := scriptArchive(t, map[string]string{"root/a.sh": "actual HTTP bytes"})
	settings := ScriptRepositorySettings{URL: "https://github.com/owner/repo", Branch: "main"}
	var status atomic.Int32
	status.Store(200)
	var redirect atomic.Bool
	rss := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/owner/repo/zip/refs/heads/main" {
			t.Error(r.URL)
		}
		if redirect.Load() {
			http.Redirect(w, r, "http://127.0.0.1/private", 302)
			return
		}
		w.WriteHeader(int(status.Load()))
		w.Write(archive)
	})
	scripts, err := fetchScriptRepository(context.Background(), rss.client, settings)
	if err != nil {
		t.Fatal(err)
	}
	if raw, _ := scripts.Raw("a.sh"); string(raw) != "actual HTTP bytes" {
		t.Fatal(string(raw))
	}
	status.Store(404)
	if _, err := fetchScriptRepository(context.Background(), rss.client, settings); !errors.Is(err, ErrRepositoryFetch) {
		t.Fatal(err)
	}
	status.Store(200)
	redirect.Store(true)
	if _, err := fetchScriptRepository(context.Background(), rss.client, settings); !errors.Is(err, ErrRepositoryFetch) {
		t.Fatal("private redirect", err)
	}
}

// Preserve production URL/redirect validation while routing TLS to one private
// fixture. This transport override is test-only; runtime keeps rssDial.
func localScriptRepositoryHTTP(t *testing.T, handler http.HandlerFunc) *RSS {
	t.Helper()
	server := httptest.NewTLSServer(handler)
	t.Cleanup(server.Close)
	s := newRSS(nil)
	transport := s.client.Transport.(*http.Transport)
	transport.TLSClientConfig = server.Client().Transport.(*http.Transport).TLSClientConfig.Clone()
	transport.TLSClientConfig.ServerName = "127.0.0.1"
	transport.DialContext = func(ctx context.Context, network, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
	}
	t.Cleanup(transport.CloseIdleConnections)
	return s
}
