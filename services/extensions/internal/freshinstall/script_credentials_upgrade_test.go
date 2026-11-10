package freshinstall

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"net/url"
	"os"
	"path/filepath"
	"testing"
)

func TestScriptCredentialsUpgradeCLIActualPostgres(t *testing.T) {
	dsn := os.Getenv("LMM_SCRIPT_CREDENTIALS_CLI_TEST_ADMIN_URL")
	if dsn == "" {
		t.Skip("requires a dedicated empty local PostgreSQL cluster")
	}
	u, err := url.Parse(dsn)
	if err != nil || u.Hostname() != "127.0.0.1" || u.Path != "/postgres" {
		t.Fatal("private loopback fixture required")
	}
	admin, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer admin.Close()
	var empty bool
	if err = admin.QueryRow(`SELECT NOT EXISTS(SELECT FROM pg_database WHERE datname NOT IN('postgres','template0','template1'))`).Scan(&empty); err != nil || !empty {
		t.Fatal("empty cluster required")
	}
	for _, statement := range []string{
		`CREATE ROLE scriptcli_owner NOLOGIN`,
		`CREATE ROLE scriptcli_runtime LOGIN`,
		`CREATE ROLE scriptcli_installer LOGIN NOINHERIT PASSWORD 'private-fixture-only'`,
		`GRANT scriptcli_owner TO scriptcli_installer`,
		`CREATE DATABASE lmm_operations_cli_test OWNER scriptcli_owner`,
		`REVOKE ALL ON DATABASE lmm_operations_cli_test FROM PUBLIC`,
		`GRANT CONNECT ON DATABASE lmm_operations_cli_test TO scriptcli_installer`,
	} {
		if _, err = admin.Exec(statement); err != nil {
			t.Fatal(err)
		}
	}
	u.Path = "/lmm_operations_cli_test"
	u.User = url.UserPassword("scriptcli_installer", "private-fixture-only")
	credential := filepath.Join(t.TempDir(), "owner-url")
	if err = os.WriteFile(credential, []byte(u.String()), 0600); err != nil {
		t.Fatal(err)
	}
	_, err = Install(context.Background(), Config{Component: "operations", OwnerURLFile: credential, ExpectedDatabase: "lmm_operations_cli_test", OwnerRole: "scriptcli_owner", RuntimeRole: "scriptcli_runtime"})
	if err != nil {
		t.Fatal("install predecessor", err)
	}
	args := []string{"--owner-url-file", credential, "--database", "lmm_operations_cli_test", "--owner-role", "scriptcli_owner", "--runtime-role", "scriptcli_runtime", "--operation-id", "private-repository-upgrade-test"}
	var output bytes.Buffer
	if err = RunScriptRepositoryCredentialsUpgradeCLI(context.Background(), args, &output); err == nil {
		t.Fatal("unfenced runtime accepted")
	}
	if _, err = admin.Exec(`ALTER ROLE scriptcli_runtime NOLOGIN; REVOKE CONNECT ON DATABASE lmm_operations_cli_test FROM scriptcli_runtime`); err != nil {
		t.Fatal(err)
	}
	if err = RunScriptRepositoryCredentialsUpgradeCLI(context.Background(), append(args, "--recover"), &output); err == nil {
		t.Fatal("absent receipt recovered")
	}
	output.Reset()
	if err = RunScriptRepositoryCredentialsUpgradeCLI(context.Background(), args, &output); err != nil {
		t.Fatal("upgrade", err)
	}
	var receipt map[string]any
	if err = json.Unmarshal(output.Bytes(), &receipt); err != nil || receipt["component"] != "script_repository_credentials" || receipt["version"] != float64(1) {
		t.Fatal("wrong component receipt", err)
	}
	output.Reset()
	if err = RunScriptRepositoryCredentialsUpgradeCLI(context.Background(), append(args, "--recover"), &output); err != nil {
		t.Fatal("recovery", err)
	}
	wrong := append([]string{}, args...)
	wrong[len(wrong)-1] = "replacement-operation"
	if err = RunScriptRepositoryCredentialsUpgradeCLI(context.Background(), append(wrong, "--recover"), &output); err == nil {
		t.Fatal("recovery accepted a different operation")
	}
	db, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	if _, err = db.Exec(`SET ROLE scriptcli_owner`); err != nil {
		t.Fatal(err)
	}
	var preserved bool
	err = db.QueryRow(`SELECT
 (SELECT revision=0 AND github_key_ciphertext='' AND scripts IS NULL FROM lmm_operations.script_repository WHERE singleton) AND
 (SELECT operation_id='private-repository-upgrade-test' AND installed_by='scriptcli_installer' FROM lmm_operations.script_repository_credentials_upgrades WHERE version=1) AND
 NOT has_table_privilege('scriptcli_runtime','lmm_operations.script_repository_credentials_upgrades','SELECT,INSERT,UPDATE,DELETE')`).Scan(&preserved)
	if err != nil || !preserved {
		t.Fatal("receipt/configuration/ACL readback", err)
	}
	t.Log("actual offline CLI, original receipt recovery, wrong-operation denial and owner-only receipt passed")
}
