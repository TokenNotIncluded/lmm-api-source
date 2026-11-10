// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"errors"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
)

func repositoryTestCipher(t *testing.T, key byte) cipher.AEAD {
	t.Helper()
	block, err := aes.NewCipher([]byte(strings.Repeat(string(key), 32)))
	if err != nil {
		t.Fatal(err)
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		t.Fatal(err)
	}
	return aead
}

func TestScriptRepositoryCredentialEncryption(t *testing.T) {
	s := &ScriptRepository{credentials: repositoryTestCipher(t, 'a')}
	const token = "github_pat_offline_fixture_only"
	sealed, err := s.sealCredential(token)
	if err != nil || strings.Contains(sealed, token) {
		t.Fatal("sealing", err)
	}
	again, err := s.sealCredential(token)
	if err != nil || again == sealed {
		t.Fatal("nonce reuse", err)
	}
	if got, err := s.openCredential(sealed); err != nil || got != token {
		t.Fatal("round trip", err)
	}
	wrong := &ScriptRepository{credentials: repositoryTestCipher(t, 'b')}
	for _, invalid := range []string{sealed, "v1:garbage", "v2:" + sealed[3:]} {
		if _, err := wrong.openCredential(invalid); !errors.Is(err, ErrUnavailable) {
			t.Fatal("wrong key/version", err)
		}
	}
	for _, invalid := range []string{"a\r\nb", "a b", strings.Repeat("a", 4097)} {
		if _, err := s.sealCredential(invalid); !errors.Is(err, ErrInvalid) {
			t.Fatal("invalid credential", err)
		}
	}
	if _, err := (&ScriptRepository{}).openCredential(sealed); !errors.Is(err, ErrUnavailable) {
		t.Fatal("missing key", err)
	}
}

func TestScriptRepositoryPrivateHTTPAuthorizationAndRedirects(t *testing.T) {
	archive := scriptArchive(t, map[string]string{"repo/setup.sh": "echo offline"})
	const token = "github_pat_offline_fixture_only"
	for _, target := range []string{
		"https://codeload.github.com/owner/repo/legacy.zip/main?download=temporary",
		"https://codeload.github.com/owner/repo/legacy.zip/feature%2Fbranch?download=temporary",
		"https://codeload.github.com/owner/repo/%2e%2e/other/archive.zip",
		"https://attacker.invalid/owner/repo/archive.zip",
		"http://codeload.github.com/owner/repo/archive.zip",
		"https://codeload.github.com/other/repo/archive.zip",
		"https://codeload.github.com/owner/repo/../other/archive.zip",
		"https://codeload.github.com/owner/repo/archive.zip?token=" + token,
	} {
		t.Run(target, func(t *testing.T) {
			var calls atomic.Int32
			transport := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				if r.Host == "api.github.com" {
					if r.URL.Path != "/repos/owner/repo/zipball/main" || r.Header.Get("Authorization") != "Bearer "+token {
						t.Error("API credential or route missing")
					}
					w.Header().Set("Location", target)
					w.WriteHeader(http.StatusFound)
					return
				}
				if r.Host != "codeload.github.com" || r.Header.Get("Authorization") != "" || r.Header.Get("Cookie") != "" || r.Header.Get("Referer") != "" {
					t.Error("redirect leaked authority")
				}
				_, _ = w.Write(archive)
			})
			files, err := fetchScriptRepositoryWithToken(context.Background(), transport.client, ScriptRepositorySettings{URL: "https://github.com/owner/repo", Branch: "main"}, token)
			if strings.HasSuffix(target, "download=temporary") {
				if err != nil || calls.Load() != 2 || string(files.entries["setup.sh"].data) != "echo offline" {
					t.Fatal("private archive", err, calls.Load())
				}
			} else if !errors.Is(err, ErrRepositoryFetch) || calls.Load() != 1 {
				t.Fatal("unsafe redirect", err, calls.Load())
			}
		})
	}
}
