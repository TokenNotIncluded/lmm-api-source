// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestIoNetCredentialPurposePreservesAPP01(t *testing.T) {
	aead := repositoryTestCipher(t, 's')
	const token = "offline-provider-fixture-key"
	repo := &ScriptRepository{credentials: aead}
	old, err := repo.sealCredential(token)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(old, "v1:"))
	if err != nil {
		t.Fatal(err)
	}
	plain, err := aead.Open(nil, raw[:aead.NonceSize()], raw[aead.NonceSize():], []byte("lmm_operations.script_repository.github_key.v1"))
	if err != nil || string(plain) != token {
		t.Fatal("legacy APP01 wire AAD changed", err)
	}
	if got, err := repo.openCredential(old); err != nil || got != token {
		t.Fatal("APP01 round trip", err)
	}
	if _, err := openCredentialPurpose(aead, ioNetSettingsPurpose, old); !errors.Is(err, ErrUnavailable) {
		t.Fatal("cross-purpose decrypt", err)
	}
	sealed, err := sealCredentialPurpose(aead, ioNetSettingsPurpose, []byte(token))
	if err != nil || strings.Contains(sealed, token) {
		t.Fatal("io.net seal", err)
	}
	if _, err := repo.openCredential(sealed); !errors.Is(err, ErrUnavailable) {
		t.Fatal("APP01 accepted io.net ciphertext", err)
	}
	if _, err := openCredentialPurpose(aead, ioNetPurpose(501, "original_operation_0001"), sealed); !errors.Is(err, ErrUnavailable) {
		t.Fatal("operation purpose accepted setting", err)
	}
}

func TestIoNetActualHTTPFixedOriginRedactionAndNoRedirect(t *testing.T) {
	const token = "offline-provider-fixture-key"
	var requests atomic.Int32
	safe := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.Host != "api.io.solutions" || r.Header.Get("x-api-key") != token || r.Header.Get("Authorization") != "" {
			t.Error("credential target or header")
		}
		if r.URL.Path != "/enterprise/v1/io-cloud/caas/hardware" {
			t.Error("provider route")
		}
		if r.URL.Query().Get("redirect") == "1" {
			w.Header().Set("Location", "https://attacker.invalid/")
			w.WriteHeader(302)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"data": map[string]any{"hardware": []any{}, "api_key": token, "message": "echo " + token, "secret_env_variables": map[string]string{"secret": "hidden"}}})
	})
	s := NewIoNet(nil)
	s.client = safe.client
	got, err := s.TestConnection(context.Background(), token)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(got)
	if strings.Contains(string(raw), token) || strings.Contains(string(raw), "hidden") {
		t.Fatal("provider secret reflected")
	}
	if _, _, err := s.request(context.Background(), token, "GET", "/hardware?redirect=1", nil); !errors.Is(err, ErrIoNetUpstream) || requests.Load() != 2 {
		t.Fatal("redirect followed", err, requests.Load())
	}
}

func TestIoNetSSESampleCancelsUpstreamAfterVisiblePrefix(t *testing.T) {
	closed := make(chan struct{})
	safe := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
		defer close(closed)
		if r.Header.Get("x-api-key") != "offline-key" || r.URL.Query().Get("stream") != "stdout" {
			t.Error("log wire")
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte(": keepalive\n\ndata: actual prefix\n\n"))
		w.(http.Flusher).Flush()
		<-r.Context().Done()
	})
	s := NewIoNet(nil)
	s.client = safe.client
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	got, err := s.sampleLogs(ctx, "offline-key", "deployment-1", "container-1", "stdout", 1)
	if err != nil || got != "actual prefix" {
		t.Fatal("SSE prefix", got, err)
	}
	select {
	case <-closed:
	case <-ctx.Done():
		t.Fatal("upstream log stream not closed")
	}
}
