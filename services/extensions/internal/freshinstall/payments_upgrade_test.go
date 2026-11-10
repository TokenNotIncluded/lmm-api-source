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

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

// This fixture owns an otherwise empty, disposable PostgreSQL cluster.
func TestPaymentsSubscriptionUpgradeCLIActualPostgres(t *testing.T) {
	dsn := os.Getenv("LMM_PAYMENTS_CLI_TEST_ADMIN_URL")
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
		`CREATE ROLE subcli_owner NOLOGIN`,
		`CREATE ROLE subcli_runtime LOGIN`,
		`CREATE ROLE subcli_installer LOGIN NOINHERIT PASSWORD 'private-fixture-only'`,
		`GRANT subcli_owner TO subcli_installer`,
		`CREATE DATABASE lmm_payments_cli_test OWNER subcli_owner`,
		`REVOKE ALL ON DATABASE lmm_payments_cli_test FROM PUBLIC`,
		`GRANT CONNECT ON DATABASE lmm_payments_cli_test TO subcli_installer`,
	} {
		if _, err = admin.Exec(statement); err != nil {
			t.Fatal(err)
		}
	}
	u.Path = "/lmm_payments_cli_test"
	setup, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal(err)
	}
	_, err = setup.Exec(`SET ROLE subcli_owner;` + payments.Schema + `GRANT USAGE ON SCHEMA public TO subcli_runtime; GRANT SELECT ON payment_storage_guard TO subcli_runtime; GRANT SELECT,INSERT,UPDATE ON payment_orders,payment_events TO subcli_runtime; INSERT INTO payment_orders(id,namespace,version,snapshot) VALUES('original','epay/1/test',1,'{"id":"original","namespace":"epay/1/test"}');`)
	setup.Close()
	if err != nil {
		t.Fatal(err)
	}
	u.User = url.UserPassword("subcli_installer", "private-fixture-only")
	credential := filepath.Join(t.TempDir(), "owner-url")
	if err = os.WriteFile(credential, []byte(u.String()), 0600); err != nil {
		t.Fatal(err)
	}
	args := []string{"--owner-url-file", credential, "--database", "lmm_payments_cli_test", "--owner-role", "subcli_owner", "--runtime-role", "subcli_runtime"}
	var output bytes.Buffer
	if err = RunPaymentsSubscriptionUpgradeCLI(context.Background(), args, &output); err == nil {
		t.Fatal("unfenced login runtime accepted")
	}
	if _, err = admin.Exec(`ALTER ROLE subcli_runtime NOLOGIN`); err != nil {
		t.Fatal(err)
	}
	if err = RunPaymentsSubscriptionUpgradeCLI(context.Background(), append(args, "--recover"), &output); err == nil {
		t.Fatal("recovery installed an absent upgrade")
	}
	output.Reset()
	if err = RunPaymentsSubscriptionUpgradeCLI(context.Background(), args, &output); err != nil {
		t.Fatal(err)
	}
	var first payments.SubscriptionEPayUpgradeReceipt
	if err = json.Unmarshal(output.Bytes(), &first); err != nil || first.Status != "committed" || first.PreservedDataSHA256 == "" {
		t.Fatalf("missing commit receipt: %v", err)
	}
	output.Reset()
	if err = RunPaymentsSubscriptionUpgradeCLI(context.Background(), append(args, "--recover"), &output); err != nil {
		t.Fatal(err)
	}
	var replay payments.SubscriptionEPayUpgradeReceipt
	if err = json.Unmarshal(output.Bytes(), &replay); err != nil || replay.Status != "already_committed" || replay.PreservedDataSHA256 != first.PreservedDataSHA256 || replay.PreservedACLSHA256 != first.PreservedACLSHA256 {
		t.Fatalf("original receipt changed: %v", err)
	}
	readback, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal(err)
	}
	defer readback.Close()
	if _, err = readback.Exec(`SET ROLE subcli_owner`); err != nil {
		t.Fatal(err)
	}
	var intact bool
	err = readback.QueryRow(`SELECT
 (SELECT count(*)=1 FROM payment_orders) AND
 EXISTS(SELECT FROM payment_orders WHERE id='original' AND version=1 AND snapshot='{"id":"original","namespace":"epay/1/test"}'::jsonb) AND
 has_table_privilege('subcli_runtime','payment_subscription_inbox','SELECT') AND
 has_table_privilege('subcli_runtime','payment_subscription_inbox','INSERT') AND
 has_table_privilege('subcli_runtime','payment_subscription_inbox','UPDATE') AND
 NOT has_table_privilege('subcli_runtime','payment_subscription_inbox','DELETE') AND
 NOT has_table_privilege('subcli_runtime','payment_subscription_upgrades','INSERT,UPDATE,DELETE')`).Scan(&intact)
	if err != nil || !intact {
		t.Fatalf("post-upgrade rows/privileges differ: %v", err)
	}
	t.Log("actual CLI rejected live runtime and absent recovery; owner upgrade and original receipt replay passed")
}
