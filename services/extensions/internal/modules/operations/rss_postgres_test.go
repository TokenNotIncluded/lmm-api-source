// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"sync"
	"sync/atomic"
	"testing"
)

func TestRSSPostgresSettingsPersistenceAndConcurrentEdit(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("RSS_TEST_OWNER_DSN"), os.Getenv("RSS_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires fresh isolated RSS owner/runtime PostgreSQL fixture")
	}
	ctx := context.Background()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	runtime, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	if err := Initialize(ctx, owner, "rss_runtime"); err != nil {
		t.Fatal(err)
	}
	service, err := New(ctx, Config{Releases: runtime})
	if err != nil {
		t.Fatal(err)
	}
	settings, err := service.RSS.Settings(ctx)
	if err != nil || settings.Revision != 0 || settings.Feeds == nil || len(settings.Feeds) != 0 {
		t.Fatal("initial configuration", settings, err)
	}
	settings.Feeds = []RSSFeedConfig{{ID: "news", Name: "News", URL: "https://feeds.example/news", Enabled: true}}
	var accepted atomic.Int32
	var wg sync.WaitGroup
	for i := 0; i < 12; i++ {
		wg.Go(func() {
			if _, err := service.RSS.Save(ctx, 501, settings); err == nil {
				accepted.Add(1)
			} else if !errors.Is(err, ErrConflict) {
				t.Error(err)
			}
		})
	}
	wg.Wait()
	if accepted.Load() != 1 {
		t.Fatal("concurrent changes were lost", accepted.Load())
	}
	if err := runtime.Close(); err != nil {
		t.Fatal(err)
	}
	runtime, err = sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	reloaded, err := New(ctx, Config{Releases: runtime})
	if err != nil {
		t.Fatal(err)
	}
	saved, err := reloaded.RSS.Settings(ctx)
	if err != nil || saved.Revision != 1 || len(saved.Feeds) != 1 || saved.Feeds[0] != settings.Feeds[0] {
		t.Fatal("durable configuration", saved, err)
	}
	saved.Feeds[0].URL = "http://127.0.0.1/private"
	if _, err := reloaded.RSS.Save(ctx, 501, saved); !errors.Is(err, ErrInvalid) {
		t.Fatal("invalid configuration was persisted", err)
	}
	for _, query := range []string{"DELETE FROM lmm_operations.rss_config", "INSERT INTO lmm_operations.rss_config(singleton) VALUES(false)", "TRUNCATE lmm_operations.rss_config", "ALTER TABLE lmm_operations.rss_config ADD COLUMN unsafe text"} {
		if _, err := runtime.ExecContext(ctx, query); err == nil {
			t.Fatal("runtime exceeded RSS permissions", query)
		}
	}
	if _, err := owner.ExecContext(ctx, "GRANT DELETE ON lmm_operations.rss_config TO rss_runtime"); err != nil {
		t.Fatal(err)
	}
	if err := Check(ctx, runtime); err == nil {
		t.Fatal("excess RSS grant accepted")
	}
}
