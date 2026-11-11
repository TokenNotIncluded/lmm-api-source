package freshinstall

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/composition"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/support"
	"github.com/lib/pq"
)

// The caller supplies a disposable empty local cluster, never an application DB.
func TestCommerceInstallCLIActualPostgres(t *testing.T) {
	dsn := os.Getenv("LMM_COMMERCE_INSTALL_TEST_ADMIN_URL")
	if dsn == "" {
		t.Skip("requires a dedicated empty local PostgreSQL cluster")
	}
	base, err := url.Parse(dsn)
	if err != nil || base.Hostname() != "127.0.0.1" || base.Path != "/postgres" {
		t.Fatal("private loopback fixture required")
	}
	admin, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer admin.Close()
	var empty bool
	if err = admin.QueryRow(`SELECT NOT EXISTS(SELECT FROM pg_database WHERE datname NOT IN('postgres','template0','template1'))`).Scan(&empty); err != nil || !empty {
		t.Fatal("empty synthetic cluster required", err)
	}
	for _, component := range []string{"store", "support"} {
		owner, installer, runtime := "commerce_"+component+"_owner", "commerce_"+component+"_installer", "commerce_"+component+"_runtime"
		for _, statement := range []string{
			"CREATE ROLE " + owner + " NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS",
			"CREATE ROLE " + installer + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic-only'",
			"CREATE ROLE " + runtime + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic-only'",
			"GRANT " + owner + " TO " + installer + " WITH INHERIT FALSE, SET TRUE",
			"CREATE DATABASE lmm_" + component + "_prod OWNER " + owner,
			"REVOKE ALL ON DATABASE lmm_" + component + "_prod FROM PUBLIC",
			"GRANT CONNECT ON DATABASE lmm_" + component + "_prod TO " + installer + "," + runtime,
		} {
			if _, err = admin.Exec(statement); err != nil {
				t.Fatal(err)
			}
		}
	}
	for _, statement := range []string{`CREATE DATABASE lmm_core_synthetic_test`, `REVOKE ALL ON DATABASE lmm_core_synthetic_test FROM PUBLIC`} {
		if _, err = admin.Exec(statement); err != nil {
			t.Fatal(err)
		}
	}
	runtimeDSNs := map[string]string{}
	for _, component := range []string{"store", "support"} {
		t.Run(component, func(t *testing.T) {
			owner, installer, runtime := "commerce_"+component+"_owner", "commerce_"+component+"_installer", "commerce_"+component+"_runtime"
			dbname := "lmm_" + component + "_prod"
			other := "lmm_store_prod"
			if component == "store" {
				other = "lmm_support_prod"
			}
			credentialURL := *base
			credentialURL.Path = "/" + dbname
			credentialURL.User = url.UserPassword(installer, "synthetic-only")
			file := filepath.Join(t.TempDir(), "installer-url")
			if err = os.WriteFile(file, []byte(credentialURL.String()), 0600); err != nil {
				t.Fatal(err)
			}
			args := []string{"--component", component, "--owner-url-file", file, "--database", dbname, "--owner-role", owner, "--runtime-role", runtime}
			var output bytes.Buffer
			if _, err = admin.Exec("GRANT CONNECT ON DATABASE " + other + " TO " + runtime); err != nil {
				t.Fatal(err)
			}
			if err = RunCLI(context.Background(), args, &output); err == nil || output.Len() != 0 {
				t.Fatal("foreign app CONNECT accepted")
			}
			if _, err = admin.Exec("REVOKE CONNECT ON DATABASE " + other + " FROM " + runtime); err != nil {
				t.Fatal(err)
			}
			if _, err = admin.Exec("GRANT CONNECT ON DATABASE lmm_core_synthetic_test TO " + runtime); err != nil {
				t.Fatal(err)
			}
			if err = RunCLI(context.Background(), args, &output); err == nil {
				t.Fatal("Core CONNECT accepted")
			}
			if _, err = admin.Exec("REVOKE CONNECT ON DATABASE lmm_core_synthetic_test FROM " + runtime); err != nil {
				t.Fatal(err)
			}
			if err = RunCLI(context.Background(), append(args, "--encryption-key-file", file), &output); err == nil {
				t.Fatal("market key flag accepted for commerce")
			}
			if err = RunCLI(context.Background(), args, &output); err != nil {
				t.Fatal("normal installer CLI", err)
			}
			var receipt Receipt
			if err = json.Unmarshal(output.Bytes(), &receipt); err != nil {
				t.Fatal(err)
			}
			plan, err := compiled(component)
			if err != nil {
				t.Fatal(err)
			}
			digest := sha256.Sum256([]byte(plan.schema))
			if receipt.Component != component || receipt.Database != dbname || receipt.SchemaSHA256 != hex.EncodeToString(digest[:]) {
				t.Fatal("wrong compiled schema receipt")
			}
			output.Reset()
			if err = RunCLI(context.Background(), args, &output); err == nil || output.Len() != 0 {
				t.Fatal("nonempty target accepted")
			}
			credentialURL.User = url.UserPassword(runtime, "synthetic-only")
			runtimeDSNs[component] = credentialURL.String()
			db, err := sql.Open("postgres", credentialURL.String())
			if err != nil {
				t.Fatal(err)
			}
			defer db.Close()
			var actual string
			if err = db.QueryRow(`SELECT current_user`).Scan(&actual); err != nil || actual != runtime {
				t.Fatal("actual runtime login", err)
			}
			var unsafe bool
			if err = db.QueryRow(`SELECT has_database_privilege(current_user,current_database(),'CREATE,TEMP') OR has_schema_privilege(current_user,$1,'CREATE') OR EXISTS(SELECT FROM pg_database WHERE datname<>current_database() AND (datname LIKE '%core%' OR datname LIKE 'lmm\_%' ESCAPE '\') AND has_database_privilege(current_user,oid,'CONNECT'))`, "lmm_"+component).Scan(&unsafe); err != nil || unsafe {
				t.Fatal("runtime boundary", err)
			}
			var tableCount int
			if err = db.QueryRow(`SELECT count(*) FROM pg_tables WHERE schemaname=$1`, "lmm_"+component).Scan(&tableCount); err != nil {
				t.Fatal(err)
			}
			expected := 17
			if component == "support" {
				expected = 4
			}
			if tableCount != expected {
				t.Fatalf("complete schema tables %d != %d", tableCount, expected)
			}
			for _, foreign := range []string{other, "lmm_core_synthetic_test"} {
				deniedURL := credentialURL
				deniedURL.Path = "/" + foreign
				denied, err := sql.Open("postgres", deniedURL.String())
				if err != nil {
					t.Fatal(err)
				}
				err = denied.Ping()
				denied.Close()
				var pgerr *pq.Error
				if !errors.As(err, &pgerr) || pgerr.Code != "42501" {
					t.Fatalf("foreign physical connect accepted: %v", err)
				}
			}
			if component == "store" {
				repo, err := store.NewPostgres(db)
				if err != nil {
					t.Fatal(err)
				}
				if err = repo.Within(context.Background(), "synthetic-shop", func(tx store.Tx) error {
					if e := tx.Put("commands", "synthetic-command", map[string]int{"revision": 1}); e != nil {
						return e
					}
					var got map[string]int
					if e := tx.Get("commands", "synthetic-command", &got); e != nil {
						return e
					}
					if got["revision"] != 1 {
						return fmt.Errorf("repository readback")
					}
					return nil
				}); err != nil {
					t.Fatal("runtime store repository", err)
				}
				if _, err = db.Exec(`INSERT INTO lmm_store.cart_items VALUES(1,repeat('a',64),'product','variant',1,1,1); UPDATE lmm_store.cart_items SET quantity=2; DELETE FROM lmm_store.cart_items; INSERT INTO lmm_store.favorites VALUES(1,'product',1); DELETE FROM lmm_store.favorites`); err != nil {
					t.Fatal("runtime collection CRUD", err)
				}
				if _, err = db.Exec(`INSERT INTO lmm_store.variants VALUES('synthetic-shop','invalid-price','{"price":{"unit":"credits","amount_credits":-1}}')`); err == nil {
					t.Fatal("typed price constraint absent")
				}
			} else {
				repo, err := support.NewPostgres(db)
				if err != nil {
					t.Fatal(err)
				}
				if err = repo.Within(context.Background(), "synthetic-shop", func(tx support.Tx) error {
					return tx.Put("commands", "synthetic-command", map[string]int{"revision": 1})
				}); err != nil {
					t.Fatal("runtime support repository", err)
				}
			}
			if _, err = db.Exec("DELETE FROM lmm_" + component + ".commands"); err == nil {
				t.Fatal("unapproved DELETE accepted")
			}
			if _, err = db.Exec("CREATE TABLE lmm_" + component + ".runtime_ddl(id int)"); err == nil {
				t.Fatal("runtime DDL accepted")
			}
			output.Reset()
			if err = os.WriteFile(file, []byte(credentialURL.String()), 0600); err != nil {
				t.Fatal(err)
			}
			if err = RunCLI(context.Background(), args, &output); err == nil {
				t.Fatal("runtime acted as installer")
			}
		})
	}
	// Exercise the normal production pool-opening and exact runtime ACL checks.
	dir := t.TempDir()
	tokenFile := filepath.Join(dir, "identity-token")
	if err = os.WriteFile(tokenFile, []byte(strings.Repeat("a", 64)), 0600); err != nil {
		t.Fatal(err)
	}
	storage := map[string]composition.StorageConfig{}
	for _, component := range []string{"store", "support"} {
		file := filepath.Join(dir, component+"-runtime-url")
		if err = os.WriteFile(file, []byte(runtimeDSNs[component]), 0600); err != nil {
			t.Fatal(err)
		}
		storage[component] = composition.StorageConfig{Component: component, Database: "lmm_" + component + "_prod", URLFile: file}
	}
	composed, err := composition.New(context.Background(), composition.Config{
		Selected: []string{"store", "support"}, Socket: "/tmp/lmm-commerce-install-core.sock", TokenFile: tokenFile,
		HostCredential: []byte(strings.Repeat("b", 64)), Store: storage["store"], Support: storage["support"],
	})
	if err != nil {
		t.Fatal("normal runtime composition with installer grants", err)
	}
	defer composed.Close()
	if composed.Store == nil || composed.Support == nil {
		t.Fatal("normal composed commerce readers missing")
	}

}
