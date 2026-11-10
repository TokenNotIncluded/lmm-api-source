// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"errors"
	"net/http"
	"os"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestScriptRepositoryCredentialPostgresUpgradePersistenceAndFailure(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("SCRIPT_CREDENTIAL_TEST_OWNER_DSN"), os.Getenv("SCRIPT_CREDENTIAL_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh isolated credential PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	owner.SetMaxOpenConns(1)
	if err := Initialize(ctx, owner, "script_credential_runtime"); err != nil {
		t.Fatal(err)
	}
	if _, err := owner.Exec(`UPDATE lmm_operations.script_repository SET revision=4,repository_url='https://github.com/owner/repo',branch='main',scripts='{"saved.sh":"b2xk"}',snapshot_repository_url='https://github.com/owner/repo',snapshot_branch='main',snapshot_pulled_at=now(); UPDATE lmm_operations.rss_config SET revision=7; INSERT INTO lmm_operations.publications(version,revision,content,published_at,published_by) VALUES('retained',1,'release',1,501);INSERT INTO lmm_operations.reads VALUES(502,1,1)`); err != nil {
		t.Fatal(err)
	}
	const operation = "script_credential_original_001"
	run := func(id string, recover bool) error {
		tx, e := owner.BeginTx(ctx, nil)
		if e != nil {
			return e
		}
		defer tx.Rollback()
		if e = UpgradeScriptRepositoryCredentials(ctx, tx, "script_credential_runtime", id, recover); e != nil {
			return e
		}
		return tx.Commit()
	}
	if err := run(operation, false); !errors.Is(err, ErrUnavailable) {
		t.Fatal("LOGIN fence", err)
	}
	if _, err := owner.Exec(`ALTER ROLE script_credential_runtime NOLOGIN`); err != nil {
		t.Fatal(err)
	}
	if err := run(operation, true); !errors.Is(err, ErrNotFound) {
		t.Fatal("recovery must not start upgrade", err)
	}
	// A transaction rollback must not leave a partially installed extension.
	tx, err := owner.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := UpgradeScriptRepositoryCredentials(ctx, tx, "script_credential_runtime", operation, false); err != nil {
		t.Fatal(err)
	}
	if err := tx.Rollback(); err != nil {
		t.Fatal(err)
	}
	if err := run(operation, true); !errors.Is(err, ErrNotFound) {
		t.Fatal("rolled back receipt", err)
	}
	if err := run(operation, false); err != nil {
		t.Fatal("upgrade", err)
	}
	if err := run(operation, true); err != nil {
		t.Fatal("original receipt recovery", err)
	}
	if err := run("script_credential_different_001", true); !errors.Is(err, ErrConflict) {
		t.Fatal("foreign operation", err)
	}
	if _, err := owner.Exec(`ALTER ROLE script_credential_runtime LOGIN`); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	service, err := New(ctx, Config{Releases: db})
	if err != nil {
		t.Fatal("upgraded runtime preflight", err)
	}
	s := service.Repository
	if raw, err := s.Raw(ctx, "saved.sh"); err != nil || string(raw) != "old" {
		t.Fatal("retained snapshot", err)
	}
	for _, query := range []string{`UPDATE lmm_operations.contract SET version=3`, `UPDATE lmm_operations.script_repository_credentials_upgrades SET operation_id='tamper'`, `ALTER TABLE lmm_operations.script_repository ADD COLUMN forbidden text`, `SELECT * FROM lmm_operations.script_repository_credentials_upgrades`} {
		if _, err := db.Exec(query); err == nil {
			t.Fatal("runtime authority widened")
		}
	}
	var retained bool
	if owner.QueryRow(`SELECT (SELECT revision=7 FROM lmm_operations.rss_config) AND (SELECT content='release' FROM lmm_operations.publications WHERE version='retained') AND EXISTS(SELECT FROM lmm_operations.reads WHERE user_id=502)`).Scan(&retained) != nil || !retained {
		t.Fatal("unrelated records changed")
	}
	key := []byte(strings.Repeat("a", 32))
	if err := s.WithCredentialKey(ctx, key); err != nil {
		t.Fatal(err)
	}
	settings, err := s.Settings(ctx)
	if err != nil || settings.Revision != 4 || settings.GitHubKeySet {
		t.Fatal("preserved settings", err)
	}
	const first = "github_pat_offline_first_credential"
	settings, err = s.SaveWithCredentials(ctx, 501, settings, first, false)
	if err != nil || !settings.GitHubKeySet {
		t.Fatal("write", err)
	}
	var ciphertext string
	if db.QueryRow(`SELECT github_key_ciphertext FROM lmm_operations.script_repository`).Scan(&ciphertext) != nil || !strings.HasPrefix(ciphertext, "v1:") || strings.Contains(ciphertext, first) {
		t.Fatal("plaintext persistence")
	}
	rebuilt, err := New(ctx, Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	if err := rebuilt.Repository.WithCredentialKey(ctx, key); err != nil {
		t.Fatal(err)
	}
	if got, err := rebuilt.Repository.credential(ctx, settings.Revision); err != nil || got != first {
		t.Fatal("restart recovery", err)
	}
	settings, err = s.Save(ctx, 501, settings)
	if err != nil || !settings.GitHubKeySet {
		t.Fatal("blank preserves key", err)
	}
	if _, err := s.SaveWithCredentials(ctx, 501, ScriptRepositorySettings{Revision: 4, URL: settings.URL, Branch: settings.Branch}, "github_pat_stale", false); !errors.Is(err, ErrConflict) {
		t.Fatal("stale credential CAS", err)
	}
	const replacement = "github_pat_offline_replacement"
	settings, err = s.SaveWithCredentials(ctx, 501, settings, replacement, false)
	if err != nil {
		t.Fatal(err)
	}
	var calls atomic.Int32
	var fail atomic.Bool
	var block atomic.Bool
	arrived, release := make(chan struct{}, 1), make(chan struct{})
	defer func() {
		select {
		case <-release:
		default:
			close(release)
		}
	}()
	archive := scriptArchive(t, map[string]string{"repo/new.sh": "new"})
	transport := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if fail.Load() {
			w.WriteHeader(http.StatusForbidden)
			return
		}
		if r.Host == "api.github.com" {
			if r.Header.Get("Authorization") != "Bearer "+replacement {
				t.Error("replacement not used")
			}
			if block.Load() {
				arrived <- struct{}{}
				<-release
			}
			w.Header().Set("Location", "https://codeload.github.com/owner/repo/legacy.zip/main")
			w.WriteHeader(http.StatusFound)
			return
		}
		if r.Header.Get("Authorization") != "" {
			t.Error("download credential leak")
		}
		_, _ = w.Write(archive)
	})
	s.client = transport.client
	missing := NewScriptRepository(db)
	missing.client = transport.client
	if _, err := missing.Pull(ctx, 501); !errors.Is(err, ErrUnavailable) || calls.Load() != 0 {
		t.Fatal("missing key must not fetch", err)
	}
	wrong := NewScriptRepository(db)
	if err := wrong.WithCredentialKey(ctx, []byte(strings.Repeat("b", 32))); err != nil {
		t.Fatal(err)
	}
	wrong.client = transport.client
	if _, err := wrong.Pull(ctx, 501); !errors.Is(err, ErrUnavailable) || calls.Load() != 0 {
		t.Fatal("wrong key must not fetch", err)
	}
	fail.Store(true)
	if _, err := s.Pull(ctx, 501); !errors.Is(err, ErrRepositoryFetch) {
		t.Fatal("upstream failure", err)
	}
	if raw, err := s.Raw(ctx, "saved.sh"); err != nil || string(raw) != "old" {
		t.Fatal("failure lost snapshot", err)
	}
	fail.Store(false)
	settings, err = s.Pull(ctx, 501)
	if err != nil || !settings.GitHubKeySet {
		t.Fatal("private pull commit", err)
	}
	if raw, err := missing.Raw(ctx, "new.sh"); err != nil || string(raw) != "new" {
		t.Fatal("private snapshot remains publicly readable without key", err)
	}
	block.Store(true)
	result := make(chan error, 1)
	go func() { _, err := s.Pull(ctx, 501); result <- err }()
	select {
	case <-arrived:
	case err := <-result:
		t.Fatal("pull failed before barrier", err)
	case <-time.After(8 * time.Second):
		t.Fatal("private HTTP barrier timed out")
	}
	settings, err = s.SaveWithCredentials(ctx, 501, settings, "", true)
	if err != nil || settings.GitHubKeySet {
		t.Fatal("clear", err)
	}
	close(release)
	if err := <-result; !errors.Is(err, ErrConflict) {
		t.Fatal("late private pull overwrote snapshot", err)
	}
	if raw, err := s.Raw(ctx, "new.sh"); err != nil || string(raw) != "new" {
		t.Fatal("CAS lost prior snapshot", err)
	}
	if _, err := s.PullRevision(ctx, 501, settings.Revision-1); !errors.Is(err, ErrConflict) {
		t.Fatal("permission revision fence", err)
	}
	block.Store(false)
	settings, err = s.Pull(ctx, 501)
	if err != nil {
		t.Fatal("public pull after clear", err)
	}
	if raw, err := s.Raw(ctx, "new.sh"); err != nil || string(raw) != "new" {
		t.Fatal("public snapshot", err)
	}
	if err := db.QueryRow(`SELECT github_key_ciphertext FROM lmm_operations.script_repository`).Scan(&ciphertext); err != nil || ciphertext != "" {
		t.Fatal("clear not durable", err)
	}
}
