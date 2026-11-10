// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

// The runner must supply an empty isolated owner database and a restricted
// LOGIN role named ionet_runtime. No provider or production credentials are used.
func TestIoNetPostgresSettingsOriginalReceiptAndUnknownFence(t *testing.T) {
	ownerDSN, runtimeDSN := os.Getenv("IONET_TEST_OWNER_DSN"), os.Getenv("IONET_TEST_RUNTIME_DSN")
	if ownerDSN == "" || runtimeDSN == "" {
		t.Skip("requires isolated io.net PostgreSQL fixture")
	}
	ctx, stop := context.WithTimeout(context.Background(), 20*time.Second)
	defer stop()
	owner, err := sql.Open("postgres", ownerDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer owner.Close()
	if err := Initialize(ctx, owner, "ionet_runtime"); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("postgres", runtimeDSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	service, err := New(ctx, Config{Releases: db})
	if err != nil {
		t.Fatal("runtime preflight", err)
	}
	s := service.IoNet
	master := []byte(strings.Repeat("i", 32))
	if err := s.WithCredentialKey(ctx, master); err != nil {
		t.Fatal(err)
	}
	settings, err := s.SaveSettings(ctx, 501, 0, true, "offline-original-key", false)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.SaveSettings(ctx, 501, 0, true, "stale-key", false); !errors.Is(err, ErrConflict) {
		t.Fatal("stale CAS", err)
	}
	var encrypted string
	if err := db.QueryRow(`SELECT api_key_ciphertext FROM lmm_operations.ionet_config`).Scan(&encrypted); err != nil || strings.Contains(encrypted, "offline-original-key") || !strings.HasPrefix(encrypted, "v1:") {
		t.Fatal("encrypted persistence", err)
	}
	settings, err = s.SaveSettings(ctx, 501, settings.Revision, true, "", false)
	if err != nil || !settings.Configured {
		t.Fatal("blank preserves", err)
	}
	restarted := NewIoNet(db)
	if err := restarted.WithCredentialKey(ctx, master); err != nil {
		t.Fatal(err)
	}
	if _, key, err := restarted.providerKey(ctx); err != nil || key != "offline-original-key" {
		t.Fatal("restart decrypt", err)
	}
	for _, statement := range []string{`SELECT * FROM lmm_operations.ionet_upgrades`, `UPDATE lmm_operations.contract SET version=3`, `DELETE FROM lmm_operations.ionet_operations`, `ALTER TABLE lmm_operations.ionet_config ADD COLUMN forbidden text`} {
		if _, err := db.Exec(statement); err == nil {
			t.Fatal("runtime authority widened")
		}
	}
	var posts, reads atomic.Int32
	var reject atomic.Bool
	arrived, release := make(chan struct{}, 1), make(chan struct{})
	defer func() {
		select {
		case <-release:
		default:
			close(release)
		}
	}()
	transport := localScriptRepositoryHTTP(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodGet {
			reads.Add(1)
			if r.Header.Get("x-api-key") != "offline-original-key" {
				t.Error("recovery used rotated key")
			}
			_, _ = w.Write([]byte(`{"data":{"id":"deployment-1","status":"running"}}`))
			return
		}
		posts.Add(1)
		if reject.Load() {
			w.WriteHeader(503)
			return
		}
		select {
		case arrived <- struct{}{}:
		default:
		}
		<-release
		_, _ = w.Write([]byte(`{"deployment_id":"deployment-1","status":"deploying"}`))
	})
	s.client = transport.client
	intent, err := ParseIoNetMutation("create", "", json.RawMessage(`{"resource_private_name":"isolated-fixture","duration_hours":1,"gpus_per_container":1,"hardware_id":1,"location_ids":[1],"container_config":{"replica_count":1,"secret_env_variables":{"secret":"fixture-secret"}},"registry_config":{"image_url":"fixture.invalid/image","registry_secret":"fixture-registry-secret"}}`))
	if err != nil {
		t.Fatal(err)
	}
	call := func(ctx context.Context, key string) (IoNetOperation, error) {
		return s.Mutation(ctx, 501, key, "create", "", intent.Resource, intent.Method, intent.Endpoint, intent.Body)
	}
	requestCtx, cancel := context.WithCancel(ctx)
	completed := make(chan IoNetOperation, 1)
	failure := make(chan error, 1)
	go func() { op, err := call(requestCtx, "original_operation_0001"); completed <- op; failure <- err }()
	select {
	case <-arrived:
	case <-ctx.Done():
		t.Fatal("provider request not reached")
	}
	duplicate, err := call(ctx, "original_operation_0001")
	if err != nil || duplicate.State != "unknown" || posts.Load() != 1 {
		t.Fatal("concurrent replay", err, posts.Load())
	}
	cancel()
	close(release)
	original := <-completed
	if err := <-failure; err != nil || original.State != "succeeded" {
		t.Fatal("disconnected original receipt", err, original.State)
	}
	restored, err := restarted.Recover(ctx, 501, original.Key)
	if err != nil || restored.State != "succeeded" || posts.Load() != 1 {
		t.Fatal("persisted recovery", err)
	}
	if _, err := s.Operation(ctx, 502, original.Key); !errors.Is(err, ErrNotFound) {
		t.Fatal("owner leak", err)
	}
	tombstone, err := s.CancelMissing(ctx, 501, "cancelled_original_0001")
	if err != nil || tombstone.State != "cancelled" {
		t.Fatal("cancel fence", err)
	}
	if _, err := call(ctx, tombstone.Key); !errors.Is(err, ErrConflict) || posts.Load() != 1 {
		t.Fatal("late cancelled dispatch", err)
	}
	reject.Store(true)
	uncertain, err := s.Mutation(ctx, 501, "unknown_extension_0001", "extend", "deployment-1", "deployment:deployment-1", "POST", "/deployment/deployment-1/extend", json.RawMessage(`{"duration_hours":1}`))
	if err != nil || uncertain.State != "unknown" {
		t.Fatal("ambiguous provider error", err)
	}
	settings, err = s.SaveSettings(ctx, 501, settings.Revision, true, "offline-rotated-key", false)
	if err != nil {
		t.Fatal(err)
	}
	settings, err = s.SaveSettings(ctx, 501, settings.Revision, false, "", true)
	if err != nil || settings.Configured {
		t.Fatal("clear", err)
	}
	resumed, err := s.Recover(ctx, 501, uncertain.Key)
	if err != nil || resumed.State != "unknown" || posts.Load() != 2 || reads.Load() != 1 {
		t.Fatal("read-only frozen credential recovery", err)
	}
	if _, err := s.CancelMissing(ctx, 501, uncertain.Key); err != nil {
		t.Fatal(err)
	}
	op, err := s.Operation(ctx, 501, uncertain.Key)
	if err != nil || op.State != "unknown" {
		t.Fatal("cancel forged remote cancellation", err)
	}
	// Re-enable to prove the durable resource fence, rather than disabled config,
	// prevents a different operation key from dispatching the same remote action.
	if _, err := s.SaveSettings(ctx, 501, settings.Revision, true, "offline-rotated-key", false); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Mutation(ctx, 501, "new_extension_key_0002", "extend", "deployment-1", "deployment:deployment-1", "POST", "/deployment/deployment-1/extend", json.RawMessage(`{"duration_hours":1}`)); !errors.Is(err, ErrConflict) || posts.Load() != 2 {
		t.Fatal("unknown fence", err)
	}
	pending, err := s.Pending(ctx, 501)
	if err != nil || len(pending) != 1 || pending[0].Key != uncertain.Key {
		t.Fatal("pending handles", err)
	}
	other, err := s.Pending(ctx, 502)
	if err != nil || len(other) != 0 {
		t.Fatal("pending owner isolation", err)
	}
	var cleartext bool
	if err := owner.QueryRow(`SELECT EXISTS(SELECT FROM lmm_operations.ionet_operations WHERE request_ciphertext LIKE '%fixture-secret%' OR request_ciphertext LIKE '%offline-original-key%' OR result::text LIKE '%fixture-registry-secret%')`).Scan(&cleartext); err != nil || cleartext {
		t.Fatal("journal secret persisted plaintext", err)
	}
}
