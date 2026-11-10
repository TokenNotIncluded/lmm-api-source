// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"errors"
	"net/http"
	"os"
	"sync"
	"sync/atomic"
	"testing"
)

func TestScriptRepositoryPostgresPersistenceAtomicPullAndConflict(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("SCRIPT_REPOSITORY_TEST_OWNER_DSN"), os.Getenv("SCRIPT_REPOSITORY_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh private script repository PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	db, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if err := Initialize(ctx, owner, "script_runtime"); err != nil {
		t.Fatal(err)
	}
	service, err := New(ctx, Config{Releases: db})
	if err != nil {
		t.Fatal(err)
	}
	s := service.Repository
	empty, err := s.Settings(ctx)
	if err != nil || empty.Revision != 0 || empty.URL != "" || empty.Branch != "main" {
		t.Fatal(empty, err)
	}
	if _, err := s.Pull(ctx, 501); !errors.Is(err, ErrInvalid) {
		t.Fatal("unconfigured pull", err)
	}
	settings := ScriptRepositorySettings{URL: "https://github.com/owner/repo", Branch: "main"}
	var accepted atomic.Int32
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Go(func() {
			if _, err := s.Save(ctx, 501, settings); err == nil {
				accepted.Add(1)
			} else if !errors.Is(err, ErrConflict) {
				t.Error(err)
			}
		})
	}
	wg.Wait()
	if accepted.Load() != 1 {
		t.Fatal("lost update", accepted.Load())
	}
	archive := scriptArchive(t, map[string]string{"repo/a.sh": "A", "repo/b.ps1": "B"})
	var failure atomic.Bool
	var barrier atomic.Bool
	arrived := make(chan struct{}, 8)
	resume := make(chan struct{})
	rss := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
		if barrier.Load() {
			arrived <- struct{}{}
			<-resume
		}
		if failure.Load() {
			http.Error(w, "supplier-secret", 500)
			return
		}
		w.Write(archive)
	})
	s.client = rss.client
	first, err := s.Pull(ctx, 501)
	if err != nil || first.Revision != 2 || first.LastPulledAt == "" || first.SnapshotURL != settings.URL {
		t.Fatal(first, err)
	}
	// Reopen the pool and reconstruct the service: neither configuration nor
	// imported bytes are satisfied by process-local state.
	otherDB, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer otherDB.Close()
	restarted, err := New(ctx, Config{Releases: otherDB})
	if err != nil {
		t.Fatal(err)
	}
	reloaded, err := restarted.Repository.Settings(ctx)
	if err != nil || reloaded != first {
		t.Fatal(reloaded, err)
	}
	if raw, err := restarted.Repository.Raw(ctx, "a.sh"); err != nil || string(raw) != "A" {
		t.Fatal(string(raw), err)
	}
	changed := first
	changed.URL = "https://github.com/owner/next"
	changed, err = s.Save(ctx, 501, changed)
	if err != nil || changed.LastPulledAt != "" || changed.SnapshotURL != first.URL || changed.SnapshotPulledAt != first.LastPulledAt {
		t.Fatal("source provenance", changed, err)
	}
	failure.Store(true)
	if _, err := s.Pull(ctx, 501); !errors.Is(err, ErrRepositoryFetch) {
		t.Fatal(err)
	}
	if raw, err := s.Raw(ctx, "a.sh"); err != nil || string(raw) != "A" {
		t.Fatal("failed pull destroyed snapshot", string(raw), err)
	}
	if list, err := s.List(ctx); err != nil || len(list) != 2 || list[0].Updated != first.LastPulledAt {
		t.Fatal("retained timestamp", list, err)
	}
	failure.Store(false)
	// A real in-flight HTTP response cannot commit after a configuration save.
	barrier.Store(true)
	result := make(chan error, 1)
	go func() { _, err := s.Pull(ctx, 501); result <- err }()
	<-arrived
	changed.Branch = "new-branch"
	changed, err = s.Save(ctx, 501, changed)
	if err != nil {
		t.Fatal(err)
	}
	resume <- struct{}{}
	if err := <-result; !errors.Is(err, ErrConflict) {
		t.Fatal("stale import committed", err)
	}
	// All pulls read one revision before any response completes: one wins.
	accepted.Store(0)
	for i := 0; i < rssConcurrency; i++ {
		wg.Go(func() {
			if _, err := s.Pull(ctx, 501); err == nil {
				accepted.Add(1)
			} else if !errors.Is(err, ErrConflict) {
				t.Error(err)
			}
		})
	}
	for i := 0; i < rssConcurrency; i++ {
		<-arrived
	}
	for i := 0; i < rssConcurrency; i++ {
		resume <- struct{}{}
	}
	wg.Wait()
	barrier.Store(false)
	if accepted.Load() != 1 {
		t.Fatal("concurrent pull overwrite", accepted.Load())
	}
	before, err := s.Settings(ctx)
	if err != nil {
		t.Fatal(err)
	}
	// A genuine PostgreSQL failure must retain both the bytes and provenance.
	if _, err := owner.ExecContext(ctx, `REVOKE UPDATE ON lmm_operations.script_repository FROM script_runtime`); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Pull(ctx, 501); !errors.Is(err, ErrUnavailable) {
		t.Fatal("write failure", err)
	}
	after, err := s.Settings(ctx)
	if err != nil || before != after {
		t.Fatal("failed update changed config", before, after, err)
	}
	if raw, err := s.Raw(ctx, "b.ps1"); err != nil || string(raw) != "B" {
		t.Fatal(string(raw), err)
	}
	if _, err := owner.ExecContext(ctx, `GRANT UPDATE ON lmm_operations.script_repository TO script_runtime`); err != nil {
		t.Fatal(err)
	}
	if err := Check(ctx, db); err != nil {
		t.Fatal("restored minimal grants", err)
	}
	for _, query := range []string{`DELETE FROM lmm_operations.script_repository`, `INSERT INTO lmm_operations.script_repository(singleton) VALUES(false)`, `TRUNCATE lmm_operations.script_repository`, `ALTER TABLE lmm_operations.script_repository ADD COLUMN unsafe text`} {
		if _, err := db.ExecContext(ctx, query); err == nil {
			t.Fatal("excess runtime authority", query)
		}
	}
	if _, err := owner.ExecContext(ctx, `GRANT DELETE ON lmm_operations.script_repository TO script_runtime`); err != nil {
		t.Fatal(err)
	}
	if err := Check(ctx, db); err == nil {
		t.Fatal("excess grants accepted")
	}
}
