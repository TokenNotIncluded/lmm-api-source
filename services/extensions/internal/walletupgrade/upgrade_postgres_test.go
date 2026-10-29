// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package walletupgrade

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"io"
	"net"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"
	"github.com/lib/pq"
)

var (
	pgSequence atomic.Uint64
	pgRoot     string
	pgSocket   string
	pgPort     = "55437"
	pgAdmin    = "wallet_test_admin"
	pgPassword = "local-synthetic-wallet-test-password"
)

func TestMain(m *testing.M) {
	if _, err := exec.LookPath("initdb"); err != nil {
		fmt.Fprintln(os.Stderr, "walletupgrade PostgreSQL tests require local initdb:", err)
		os.Exit(2)
	}
	if _, err := exec.LookPath("pg_ctl"); err != nil {
		fmt.Fprintln(os.Stderr, "walletupgrade PostgreSQL tests require local pg_ctl:", err)
		os.Exit(2)
	}
	var err error
	pgRoot, err = os.MkdirTemp("/tmp", "lmm-wallet-pg-")
	if err != nil {
		fmt.Fprintln(os.Stderr, "cannot create local PostgreSQL test directory:", err)
		os.Exit(2)
	}
	pgSocket = filepath.Join(pgRoot, "s")
	data := filepath.Join(pgRoot, "d")
	if err = os.Mkdir(pgSocket, 0700); err == nil {
		err = os.WriteFile(filepath.Join(pgRoot, "pw"), []byte(pgPassword+"\n"), 0600)
	}
	if err == nil {
		err = exec.Command("initdb", "-D", data, "-U", pgAdmin, "--pwfile="+filepath.Join(pgRoot, "pw"),
			"--auth-local=scram-sha-256", "--auth-host=scram-sha-256", "--no-locale", "--encoding=UTF8").Run()
	}
	if err == nil {
		options := fmt.Sprintf("-c listen_addresses='' -c unix_socket_directories=%s -c port=%s -c log_min_error_statement=panic",
			pgSocket, pgPort)
		err = exec.Command("pg_ctl", "-D", data, "-o", options, "-l", filepath.Join(pgRoot, "postgres.log"), "-w", "start").Run()
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "cannot start local Unix-socket PostgreSQL fixture:", err)
		os.Exit(2)
	}
	code := m.Run()
	stopErr := exec.Command("pg_ctl", "-D", data, "-m", "fast", "-w", "stop").Run()
	removeErr := os.RemoveAll(pgRoot)
	if stopErr != nil || removeErr != nil {
		fmt.Fprintln(os.Stderr, "local PostgreSQL fixture cleanup failed")
		os.Exit(2)
	}
	os.Exit(code)
}

type fixture struct {
	name, owner, installer, runtime string
	ownerURL, runtimeURL            string
	installerFile                   string
	admin                           *sql.DB
}

func newFixture(t *testing.T) *fixture {
	t.Helper()
	n := pgSequence.Add(1)
	f := &fixture{
		name: fmt.Sprintf("lmm_toolmarket_wallet_%d", n), owner: fmt.Sprintf("w_owner_%d", n),
		installer: fmt.Sprintf("w_install_%d", n), runtime: fmt.Sprintf("w_runtime_%d", n),
	}
	f.admin = openDB(t, postgresURL(pgAdmin, pgPassword, "postgres", pgSocket, pgPort))
	t.Cleanup(func() { _ = f.admin.Close() })
	for _, role := range []string{f.owner, f.installer, f.runtime} {
		if _, err := f.admin.Exec(`CREATE ROLE ` + pq.QuoteIdentifier(role)); err != nil {
			t.Fatal("create isolated test role:", err)
		}
	}
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.installer) + ` LOGIN NOINHERIT PASSWORD ` + pq.QuoteLiteral("installer-test-password")); err != nil {
		t.Fatal("configure isolated installer:", err)
	}
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` LOGIN PASSWORD ` + pq.QuoteLiteral("runtime-test-password")); err != nil {
		t.Fatal("configure isolated runtime:", err)
	}
	if _, err := f.admin.Exec(`GRANT ` + pq.QuoteIdentifier(f.owner) + ` TO ` + pq.QuoteIdentifier(f.installer)); err != nil {
		t.Fatal("grant isolated installer membership:", err)
	}
	if _, err := f.admin.Exec(`CREATE DATABASE ` + pq.QuoteIdentifier(f.name) + ` OWNER ` + pq.QuoteIdentifier(f.owner)); err != nil {
		t.Fatal("create isolated test database:", err)
	}
	if _, err := f.admin.Exec(`REVOKE ALL ON DATABASE ` + pq.QuoteIdentifier(f.name) + ` FROM PUBLIC;
GRANT CONNECT ON DATABASE ` + pq.QuoteIdentifier(f.name) + ` TO ` + pq.QuoteIdentifier(f.installer) + `,` + pq.QuoteIdentifier(f.runtime)); err != nil {
		t.Fatal("set isolated database access:", err)
	}
	f.ownerURL = postgresURL(f.installer, "installer-test-password", f.name, pgSocket, pgPort)
	f.runtimeURL = postgresURL(f.runtime, "runtime-test-password", f.name, pgSocket, pgPort)
	dir := t.TempDir()
	f.installerFile = filepath.Join(dir, "installer.url")
	if err := os.WriteFile(f.installerFile, []byte(f.ownerURL), 0600); err != nil {
		t.Fatal("write private local test credential:", err)
	}
	keyFile := filepath.Join(dir, "market.key")
	if err := os.WriteFile(keyFile, []byte(strings.Repeat("k", 32)), 0600); err != nil {
		t.Fatal("write private synthetic market key:", err)
	}
	if _, err := f.admin.Exec(`ALTER DEFAULT PRIVILEGES FOR ROLE ` + pq.QuoteIdentifier(f.owner) +
		` IN SCHEMA extension_wallet GRANT SELECT,INSERT,UPDATE,DELETE ON TABLES TO ` + pq.QuoteIdentifier(f.runtime)); err != nil {
		// The namespace is created by the installer. Configure the equivalent
		// database-level default now, which applies when its schema is created.
		_, _ = f.admin.Exec(`ALTER DEFAULT PRIVILEGES FOR ROLE ` + pq.QuoteIdentifier(f.owner) +
			` GRANT SELECT,INSERT,UPDATE,DELETE ON TABLES TO ` + pq.QuoteIdentifier(f.runtime))
	}
	// freshinstall resets every runtime object grant to its compiled minimum.
	_, err := freshinstall.Install(context.Background(), freshinstall.Config{
		OwnerURLFile: f.installerFile, ExpectedDatabase: f.name, OwnerRole: f.owner,
		RuntimeRole: f.runtime, Component: "toolmarket-builtins", EncryptionKeyFile: keyFile,
	})
	if err != nil {
		t.Fatal("run real compiled fresh installer for isolated fixture:", err)
	}
	return f
}

func (f *fixture) config(id string) Config {
	return Config{OwnerURLFile: f.installerFile, Database: f.name, OwnerRole: f.owner, RuntimeRole: f.runtime, OperationID: id}
}

func postgresURL(user, password, database, socket, port string) string {
	u := &url.URL{Scheme: "postgresql", User: url.UserPassword(user, password), Path: "/" + database}
	q := url.Values{"host": {socket}, "port": {port}, "sslmode": {"disable"}}
	u.RawQuery = q.Encode()
	return u.String()
}

func openDB(t *testing.T, dsn string) *sql.DB {
	t.Helper()
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal("open local Unix-socket PostgreSQL:", err)
	}
	db.SetMaxOpenConns(1)
	if err = db.Ping(); err != nil {
		_ = db.Close()
		t.Fatal("connect local Unix-socket PostgreSQL:", err)
	}
	return db
}

func (f *fixture) asInstaller(t *testing.T) *sql.DB {
	t.Helper()
	db := openDB(t, f.ownerURL)
	if _, err := db.Exec("SET ROLE " + pq.QuoteIdentifier(f.owner)); err != nil {
		_ = db.Close()
		t.Fatal("assume isolated schema owner for fixture setup:", err)
	}
	return db
}

func (f *fixture) asRuntime(t *testing.T) *sql.DB {
	t.Helper()
	return openDB(t, f.runtimeURL)
}

func (f *fixture) storage(t *testing.T) {
	t.Helper()
	runtime := f.asRuntime(t)
	defer runtime.Close()
	var count int
	if err := runtime.QueryRow(`SELECT count(*) FROM extension_wallet.transfer_links`).Scan(&count); err != nil {
		t.Fatal("runtime cannot read preserved transfer links:", err)
	}
	var allowed bool
	if err := runtime.QueryRow(`SELECT has_table_privilege(current_user,'extension_wallet.transfer_links','SELECT')
 AND has_table_privilege(current_user,'extension_wallet.transfer_links','INSERT')
 AND NOT has_table_privilege(current_user,'extension_wallet.transfer_links','UPDATE,DELETE,TRUNCATE')
 AND NOT has_any_column_privilege(current_user,'extension_wallet.transfer_links','UPDATE,REFERENCES')
 AND NOT has_table_privilege(current_user,'extension_wallet.schema_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')`).Scan(&allowed); err != nil || !allowed {
		t.Fatal("runtime authority or owner-only journal boundary changed")
	}
}

func seed(t *testing.T, db *sql.DB, key, token string) {
	t.Helper()
	if _, err := db.Exec(`INSERT INTO extension_wallet.transfer_links(owner_user,request_key,amount_credits,token)
 VALUES(11,$1,500,$2)`, key, token); err != nil {
		t.Fatal("seed synthetic pre-existing transfer:", err)
	}
}

func setPredecessor(t *testing.T, f *fixture) {
	t.Helper()
	owner := f.asInstaller(t)
	defer owner.Close()
	if _, err := owner.Exec(`ALTER TABLE extension_wallet.transfer_links
 DROP CONSTRAINT transfer_links_request_key_check;
 ALTER TABLE extension_wallet.transfer_links
 ADD CONSTRAINT transfer_links_request_key_check CHECK(length(request_key) BETWEEN 16 AND 64)`); err != nil {
		t.Fatal("prepare exact character-count predecessor:", err)
	}
}

func assertRows(t *testing.T, f *fixture, expected int) {
	t.Helper()
	owner := f.asInstaller(t)
	defer owner.Close()
	var count int
	if err := owner.QueryRow(`SELECT count(*) FROM extension_wallet.transfer_links`).Scan(&count); err != nil || count != expected {
		t.Fatalf("transfer rows changed: got %d, want %d", count, expected)
	}
	var rows string
	if err := owner.QueryRow(`SELECT coalesce(string_agg(owner_user::text||':'||request_key||':'||amount_credits::text||':'||token,',' ORDER BY owner_user,request_key),'')
 FROM extension_wallet.transfer_links`).Scan(&rows); err != nil || strings.Contains(rows, "redacted") {
		t.Fatal("cannot read synthetic preservation evidence")
	}
}

func TestOfflineWalletUpgradeUnixPostgresAndFencedRecovery(t *testing.T) {
	f := newFixture(t)
	setPredecessor(t, f)
	runtime := f.asRuntime(t)
	key := "legacy-existing-001"
	token := strings.Repeat("a", 64)
	seed(t, runtime, key, token)
	// A live idle client still blocks an offline migration, even after its role
	// is fenced. No schema or bearer changes are allowed before it drains.
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` NOLOGIN`); err != nil {
		t.Fatal("fence runtime login:", err)
	}
	id := "wallet-upgrade-operation-0001"
	if _, err := Apply(context.Background(), f.config(id)); !errors.Is(err, ErrRejected) {
		t.Fatalf("live runtime session was not rejected: %v", err)
	}
	if _, err := Recover(context.Background(), f.config(id)); !errors.Is(err, ErrRejected) {
		t.Fatalf("recovery did not reject the live runtime session: %v", err)
	}
	runtime.Close()
	if got, err := Apply(context.Background(), f.config(id)); err != nil || got.OperationID != id || got.Upgrade != upgradeID {
		t.Fatalf("offline schema upgrade failed: %#v %v", got, err)
	}
	assertRows(t, f, 1)
	// Reopen only for assertion: the operator controls when LOGIN is restored.
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` LOGIN`); err != nil {
		t.Fatal("restore synthetic runtime login for post-upgrade assertion:", err)
	}
	f.storage(t)
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` NOLOGIN`); err != nil {
		t.Fatal("restore offline fence before replay:", err)
	}
	replay, err := Apply(context.Background(), f.config(id))
	if err != nil || replay != (Receipt{f.name, id, upgradeID}) {
		t.Fatalf("same operation replay changed its result: %#v %v", replay, err)
	}
	if _, err = Apply(context.Background(), f.config("wallet-upgrade-operation-0002")); !errors.Is(err, ErrRejected) {
		t.Fatalf("different operation ID adopted an existing schema: %v", err)
	}
	assertRows(t, f, 1)

	owner := f.asInstaller(t)
	defer owner.Close()
	longKey := strings.Repeat("界", 22) // 66 UTF-8 bytes; the prior DB CHECK accepted it.
	_, err = owner.Exec(`INSERT INTO extension_wallet.transfer_links(owner_user,request_key,amount_credits,token)
 VALUES(12,$1,500,$2)`, longKey, strings.Repeat("b", 64))
	if err == nil {
		t.Fatal("byte limit accepted an oversized historical request key")
	}
	if _, err = owner.Exec(`INSERT INTO extension_wallet.transfer_links(owner_user,request_key,amount_credits,token)
 VALUES(12,$1,500,$2)`, "legacy-unicode-界-001", strings.Repeat("c", 64)); err != nil {
		t.Fatal("byte-valid historical UTF-8 request key was not preserved by the new constraint:", err)
	}
}

func TestWalletUpgradeRejectsDriftAndUnprovenStates(t *testing.T) {
	tests := []struct {
		name       string
		setup      func(*testing.T, *fixture)
		recoverErr error
	}{
		{"runtime login not fenced", func(t *testing.T, f *fixture) { setPredecessor(t, f) }, ErrRejected},
		{"column grant drift", func(t *testing.T, f *fixture) {
			setPredecessor(t, f)
			owner := f.asInstaller(t)
			defer owner.Close()
			if _, err := owner.Exec(`GRANT UPDATE(token) ON extension_wallet.transfer_links TO ` + pq.QuoteIdentifier(f.runtime)); err != nil {
				t.Fatal("prepare column grant drift:", err)
			}
			if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` NOLOGIN`); err != nil {
				t.Fatal(err)
			}
		}, ErrRejected},
		{"target schema without operation receipt", func(t *testing.T, f *fixture) {
			if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` NOLOGIN`); err != nil {
				t.Fatal(err)
			}
		}, ErrAbsent},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			f := newFixture(t)
			test.setup(t, f)
			if _, err := Apply(context.Background(), f.config("wallet-upgrade-operation-0003")); !errors.Is(err, ErrRejected) {
				t.Fatalf("unproven or unsafe predecessor was accepted: %v", err)
			}
			if _, err := Recover(context.Background(), f.config("wallet-upgrade-operation-0003")); !errors.Is(err, test.recoverErr) {
				t.Fatalf("wrong recovery result: got %v, want %v", err, test.recoverErr)
			}
			var journal bool
			owner := f.asInstaller(t)
			defer owner.Close()
			err := owner.QueryRow(`SELECT to_regclass('extension_wallet.schema_upgrades') IS NOT NULL`).Scan(&journal)
			if err != nil {
				t.Fatal("inspect isolated migration journal:", err)
			}
			if journal {
				t.Fatal("rejected upgrade left a journal behind")
			}
		})
	}
}

func TestWalletUpgradeRejectsUnsafeInputAndRuntimeAuthority(t *testing.T) {
	f := newFixture(t)
	for _, cfg := range []Config{
		{Database: f.name, OwnerRole: f.owner, RuntimeRole: f.runtime, OperationID: "wallet-upgrade-operation-0004"},
		{OwnerURLFile: f.installerFile, Database: "lmm_core_fake", OwnerRole: f.owner, RuntimeRole: f.runtime, OperationID: "wallet-upgrade-operation-0004"},
		{OwnerURLFile: f.installerFile, Database: f.name, OwnerRole: f.runtime, RuntimeRole: f.owner, OperationID: "wallet-upgrade-operation-0004"},
		{OwnerURLFile: f.installerFile, Database: f.name, OwnerRole: f.owner, RuntimeRole: f.runtime, OperationID: "manual-hash-is-not-a-permission"},
	} {
		if _, err := Apply(context.Background(), cfg); !errors.Is(err, ErrRejected) {
			t.Fatalf("unsafe migration config accepted: %v", err)
		}
	}
	dir := t.TempDir()
	tcpFile := filepath.Join(dir, "tcp.url")
	tcpURL := postgresURL(f.installer, "installer-test-password", f.name, "127.0.0.1", pgPort)
	if err := os.WriteFile(tcpFile, []byte(tcpURL), 0600); err != nil {
		t.Fatal(err)
	}
	cfg := f.config("wallet-upgrade-operation-0005")
	cfg.OwnerURLFile = tcpFile
	if _, err := Apply(context.Background(), cfg); !errors.Is(err, ErrRejected) {
		t.Fatalf("non-Unix transport accepted: %v", err)
	}
	// The runtime role has no SET path to the installation owner.
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` NOLOGIN`); err != nil {
		t.Fatal(err)
	}
	if _, err := Apply(context.Background(), f.config("wallet-upgrade-operation-0006")); !errors.Is(err, ErrRejected) {
		t.Fatalf("current byte schema without a real predecessor was adopted: %v", err)
	}
}

func TestWalletUpgradeRecoversAfterCommitReplyIsDropped(t *testing.T) {
	f := newFixture(t)
	setPredecessor(t, f)
	if _, err := f.admin.Exec(`ALTER ROLE ` + pq.QuoteIdentifier(f.runtime) + ` NOLOGIN`); err != nil {
		t.Fatal("fence runtime login:", err)
	}
	owner := f.asInstaller(t)
	seed(t, owner, "legacy-existing-002", strings.Repeat("d", 64))
	owner.Close()

	proxy, proxyURL, done := startCommitDropProxy(t, f.ownerURL)
	_ = proxy
	cfg := f.config("wallet-upgrade-operation-0007")
	cfg.OwnerURLFile = proxyURL
	if _, err := Apply(context.Background(), cfg); !errors.Is(err, ErrUnknown) {
		t.Fatalf("dropped PostgreSQL COMMIT reply was not reported unknown: %v", err)
	}
	select {
	case <-done:
	case <-time.After(3 * time.Second):
		t.Fatal("local PostgreSQL commit-drop proxy did not close")
	}
	receipt, err := Recover(context.Background(), f.config(cfg.OperationID))
	if err != nil || receipt.OperationID != cfg.OperationID {
		t.Fatalf("read-only recovery did not find the committed original operation: %#v %v", receipt, err)
	}
	assertRows(t, f, 1)
}

func startCommitDropProxy(t *testing.T, upstreamURL string) (net.Listener, string, <-chan struct{}) {
	t.Helper()
	u, err := url.Parse(upstreamURL)
	if err != nil {
		t.Fatal("parse private Unix fixture URL:", err)
	}
	q, err := url.ParseQuery(u.RawQuery)
	if err != nil {
		t.Fatal(err)
	}
	proxyDir, err := os.MkdirTemp(pgRoot, "proxy-")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(proxyDir, ".s.PGSQL."+q.Get("port"))
	listener, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal("listen on private Unix-socket test proxy:", err)
	}
	done := make(chan struct{})
	t.Cleanup(func() { _ = listener.Close() })
	go func() {
		defer close(done)
		client, acceptErr := listener.Accept()
		if acceptErr != nil {
			return
		}
		server, dialErr := net.Dial("unix", filepath.Join(q.Get("host"), ".s.PGSQL."+q.Get("port")))
		if dialErr != nil {
			_ = client.Close()
			return
		}
		var closeOnce sync.Once
		closePair := func() {
			closeOnce.Do(func() {
				_ = client.Close()
				_ = server.Close()
				_ = listener.Close()
			})
		}
		defer closePair()
		var size [4]byte
		if _, err := io.ReadFull(client, size[:]); err != nil {
			return
		}
		n := int(size[0])<<24 | int(size[1])<<16 | int(size[2])<<8 | int(size[3])
		if n < 8 || n > 1<<20 {
			return
		}
		startup := make([]byte, n-4)
		if _, err := io.ReadFull(client, startup); err != nil {
			return
		}
		if err = writeAll(server, append(size[:], startup...)); err != nil {
			return
		}
		var wg sync.WaitGroup
		wg.Add(2)
		go func() {
			defer wg.Done()
			for {
				var header [5]byte
				if _, err := io.ReadFull(server, header[:]); err != nil {
					closePair()
					return
				}
				n := int(header[1])<<24 | int(header[2])<<16 | int(header[3])<<8 | int(header[4])
				if n < 4 || n > 16<<20 {
					closePair()
					return
				}
				body := make([]byte, n-4)
				if _, err := io.ReadFull(server, body); err != nil {
					closePair()
					return
				}
				if header[0] == 'C' && string(body) == "COMMIT\x00" {
					// PostgreSQL sent CommandComplete: the COMMIT is durable,
					// while the simulated client never receives its confirmation.
					closePair()
					return
				}
				if err = writeAll(client, append(header[:], body...)); err != nil {
					closePair()
					return
				}
			}
		}()
		go func() {
			defer wg.Done()
			_, _ = io.Copy(server, client)
			closePair()
		}()
		wg.Wait()
	}()
	u.Host = ""
	u.RawQuery = url.Values{"host": {proxyDir}, "port": {q.Get("port")}, "sslmode": {"disable"}}.Encode()
	u.ForceQuery = true
	privateURL := filepath.Join(pgRoot, "proxy.url")
	if err := os.WriteFile(privateURL, []byte(u.String()), 0600); err != nil {
		t.Fatal("write private proxy URL:", err)
	}
	return listener, privateURL, done
}

func writeAll(dst io.Writer, data []byte) error {
	for len(data) != 0 {
		n, err := dst.Write(data)
		if err != nil {
			return err
		}
		data = data[n:]
	}
	return nil
}
