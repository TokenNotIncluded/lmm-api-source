package payments

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"testing"
	"time"

	"github.com/lib/pq"
)

// These tests use the real PostgreSQL driver/schema and a controlled Core boundary.
// Core signature and financial admission remain covered by the separate RSA fixture.
func openPancakeRecoveryPG(t *testing.T) *sql.DB {
	t.Helper()
	dsn := os.Getenv("PAYMENT_TEST_DSN")
	if dsn == "" {
		t.Skip("PAYMENT_TEST_DSN must name a disposable local lmm_payments_test database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	var name string
	if err := db.QueryRow("SELECT current_database()").Scan(&name); err != nil || name != "lmm_payments_test" {
		t.Fatal("refuse non-test database", name, err)
	}
	return db
}

func pancakeRecoveryService(db *sql.DB, core *legacyFake) (*PancakeSubscriptions, *Service) {
	recovery := &PancakeSubscriptions{db: db}
	wallet := legacyService(core)
	recovery.AttachWallet(wallet)
	return recovery, wallet
}

type pancakeRecoveryRow struct {
	state, code                string
	result, payload, signature []byte
	resultNull                 bool
	attempts                   int
	next                       time.Time
	delay                      float64
}

func readPancakeRecoveryRow(t *testing.T, db *sql.DB) pancakeRecoveryRow {
	t.Helper()
	var count int
	if err := db.QueryRow("SELECT count(*) FROM payment_pancake_wallet_inbox").Scan(&count); err != nil || count != 1 {
		t.Fatal("expected exactly one original inbox row", count, err)
	}
	var row pancakeRecoveryRow
	var id string
	err := db.QueryRow(`SELECT evidence_id,state,error_code,result,result IS NULL,payload,signature,attempts,next_attempt,
		extract(epoch FROM next_attempt-clock_timestamp())::float8 FROM payment_pancake_wallet_inbox`).Scan(
		&id, &row.state, &row.code, &row.result, &row.resultNull, &row.payload, &row.signature, &row.attempts, &row.next, &row.delay)
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.Sum256([]byte(oldCallback))
	expected := fmt.Sprintf("%x", sha256.Sum256(append([]byte(fmt.Sprintf("pancake-wallet-v1:4:%x:", hash)), []byte(" original-signature ")...)))
	if id != expected || !bytes.Equal(row.payload, []byte(oldCallback)) || !bytes.Equal(row.signature, []byte(" original-signature ")) {
		t.Fatal("original evidence identity or bytes changed")
	}
	return row
}

func TestPancakeWalletRecoveryPostgres(t *testing.T) {
	db := openPancakeRecoveryPG(t)
	parent := t
	reset := func(t *testing.T) {
		t.Helper()
		_, err := db.Exec(`DROP TABLE IF EXISTS payment_pancake_wallet_inbox,payment_pancake_subscription_inbox,payment_pancake_subscription_checkouts;
			DROP FUNCTION IF EXISTS guard_pancake_subscription_recovery();` + SubscriptionPancakeSchema)
		if err != nil {
			t.Fatal(err)
		}
	}
	t.Run("not_found_backoff_process_restart_duplicate", func(t *testing.T) {
		reset(t)
		core := &legacyFake{lookupErr: ErrNotFound, acceptErr: ErrNotFound}
		_, wallet := pancakeRecoveryService(db, core)
		if err := deliverLegacy(wallet, oldCallback); !errors.Is(err, ErrNotFound) {
			t.Fatal("unknown basis must preserve NotFound", err)
		}
		first := readPancakeRecoveryRow(t, db)
		if first.state != "pending" || !first.resultNull || first.code != "retry_original" || first.attempts != 1 || first.delay <= 0 || first.delay > 2.1 {
			t.Fatalf("first retry not persisted: %+v", first)
		}
		if core.credits != 0 || core.postings != 0 {
			t.Fatal("unknown basis reached credit")
		}
		// Reopen in another OS process after the actual scheduled deadline.
		if err := db.Close(); err != nil {
			t.Fatal(err)
		}
		time.Sleep(time.Until(first.next.Add(50 * time.Millisecond)))
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		child := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestPancakeWalletRecoveryPostgresChild$", "-test.v")
		child.Env = append(os.Environ(), "LMM_PANCAKE_RECOVERY_CHILD=1")
		output, err := child.CombinedOutput()
		t.Logf("separate recovery process\n%s", output)
		if err != nil {
			t.Fatal("separate process recovery failed", err)
		}
		db = openPancakeRecoveryPG(parent)
		restarted := readPancakeRecoveryRow(t, db)
		if restarted.state != "pending" || !restarted.resultNull || restarted.code != "retry_original" || restarted.attempts != 2 || !restarted.next.After(first.next) || restarted.delay <= 0 || restarted.delay > 3.1 {
			t.Fatalf("restart did not persist retry/backoff: %+v", restarted)
		}
		_, wallet = pancakeRecoveryService(db, core)
		if err := deliverLegacy(wallet, oldCallback); !errors.Is(err, ErrNotFound) {
			t.Fatal(err)
		}
		duplicate := readPancakeRecoveryRow(t, db)
		if duplicate.attempts != 3 || duplicate.state != "pending" || !duplicate.resultNull || duplicate.code != "retry_original" || !duplicate.next.After(restarted.next) || duplicate.delay <= 0 || duplicate.delay > 5.1 {
			t.Fatalf("exact duplicate did not retry same pending original: %+v", duplicate)
		}
		if core.credits != 0 || core.postings != 0 {
			t.Fatal("unknown basis reached credit")
		}
	})
	t.Run("success_keeps_original_bytea_and_deduplicates", func(t *testing.T) {
		reset(t)
		core := &legacyFake{}
		recovery, wallet := pancakeRecoveryService(db, core)
		for i := 0; i < 2; i++ {
			if err := deliverLegacy(wallet, oldCallback); err != nil {
				t.Fatal(err)
			}
		}
		if err := recovery.recoverWallet(context.Background(), 1); err != nil {
			t.Fatal(err)
		}
		row := readPancakeRecoveryRow(t, db)
		if row.state != "accepted" || row.resultNull || !bytes.Equal(row.result, []byte("durable_original_wallet_event")) || row.attempts != 1 || row.code != "" || core.credits != 1 || core.postings != 1 {
			t.Fatalf("success/replay changed: row=%+v credits=%d postings=%d", row, core.credits, core.postings)
		}
	})
	t.Run("rejected_result_is_sql_null", func(t *testing.T) {
		reset(t)
		core := &legacyFake{lookupErr: ErrNotFound, acceptErr: ErrEvidence}
		_, wallet := pancakeRecoveryService(db, core)
		for i := 0; i < 2; i++ {
			if err := deliverLegacy(wallet, oldCallback); !errors.Is(err, ErrEvidence) {
				t.Fatal(err)
			}
		}
		row := readPancakeRecoveryRow(t, db)
		if row.state != "rejected" || !row.resultNull || row.attempts != 1 || row.code != "invalid_original" || core.credits != 0 {
			t.Fatalf("rejected evidence incorrectly persisted/retried: %+v", row)
		}
	})
	t.Run("typed_nil_check_failure_rolls_back", func(t *testing.T) {
		reset(t)
		core := &legacyFake{lookupErr: ErrNotFound, acceptErr: ErrNotFound}
		_, wallet := pancakeRecoveryService(db, core)
		if err := deliverLegacy(wallet, oldCallback); !errors.Is(err, ErrNotFound) {
			t.Fatal(err)
		}
		before := readPancakeRecoveryRow(t, db)
		tx, err := db.Begin()
		if err != nil {
			t.Fatal(err)
		}
		defer tx.Rollback()
		var typedNil []byte
		_, err = tx.Exec(`UPDATE payment_pancake_wallet_inbox SET result=$1,attempts=attempts+1`, typedNil)
		var pgError *pq.Error
		if !errors.As(err, &pgError) || pgError.Code != "23514" {
			t.Fatal("original NULL CHECK must reject typed nil encoded as bytea", err)
		}
		if err := tx.Rollback(); err != nil {
			t.Fatal(err)
		}
		after := readPancakeRecoveryRow(t, db)
		if after.attempts != before.attempts || !after.next.Equal(before.next) || after.state != before.state || !after.resultNull || after.code != before.code {
			t.Fatal("failed UPDATE changed durable original")
		}
	})
}

func TestPancakeWalletRecoveryPostgresChild(t *testing.T) {
	if os.Getenv("LMM_PANCAKE_RECOVERY_CHILD") != "1" {
		t.Skip("only spawned by the PostgreSQL process-restart regression")
	}
	db := openPancakeRecoveryPG(t)
	core := &legacyFake{lookupErr: ErrNotFound, acceptErr: ErrNotFound}
	recovery, _ := pancakeRecoveryService(db, core)
	if err := recovery.recoverWallet(context.Background(), 1); !errors.Is(err, ErrNotFound) {
		t.Fatal("restart must retry the original NotFound", err)
	}
	if core.credits != 0 || core.postings != 0 {
		t.Fatal("restart reached credit for unknown basis")
	}
}
