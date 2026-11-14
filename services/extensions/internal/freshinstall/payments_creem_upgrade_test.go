package freshinstall

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
)

// This fixture owns only its named database and roles in a private local PG18 cluster.
// Capture and actual Core fixtures may coexist in separate databases; nothing is dropped.
func TestPaymentsCreemSubscriptionUpgradeCLIActualPostgres(t *testing.T) {
	dsn := os.Getenv("LMM_PAYMENTS_CREEM_CLI_TEST_ADMIN_URL")
	if dsn == "" {
		t.Skip("requires a dedicated empty local PostgreSQL cluster")
	}
	u, err := url.Parse(dsn)
	port, portErr := 0, error(nil)
	if err == nil {
		port, portErr = strconv.Atoi(u.Query().Get("port"))
	}
	if err != nil || portErr != nil || port < 49152 || !strings.HasPrefix(u.Query().Get("host"), "/tmp/") || u.Path != "/postgres" {
		t.Fatal("private loopback fixture required")
	}
	admin, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer admin.Close()
	var empty bool
	if err = admin.QueryRow(`SELECT current_setting('server_version_num')::int BETWEEN 180000 AND 189999 AND NOT EXISTS(SELECT FROM pg_database WHERE datname='lmm_payments_creem_cli_test') AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN('creemcli_owner','creemcli_runtime','creemcli_installer'))`).Scan(&empty); err != nil || !empty {
		t.Fatal("unused named fixture database/roles in PG18 required")
	}
	for _, statement := range []string{
		`CREATE ROLE creemcli_owner NOLOGIN`,
		`CREATE ROLE creemcli_runtime LOGIN`,
		`CREATE ROLE creemcli_installer LOGIN NOINHERIT PASSWORD 'private-fixture-only'`,
		`GRANT creemcli_owner TO creemcli_installer`,
		`CREATE DATABASE lmm_payments_creem_cli_test OWNER creemcli_owner`,
		`REVOKE ALL ON DATABASE lmm_payments_creem_cli_test FROM PUBLIC`,
		`GRANT CONNECT ON DATABASE lmm_payments_creem_cli_test TO creemcli_installer`,
	} {
		if _, err = admin.Exec(statement); err != nil {
			t.Fatal(err)
		}
	}
	u.Path = "/lmm_payments_creem_cli_test"
	setup, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal(err)
	}
	_, err = setup.Exec(`SET ROLE creemcli_owner;` + payments.Schema + payments.SubscriptionEPaySchema + payments.SubscriptionStripeSchema + payments.SubscriptionPancakeSchema + `GRANT USAGE ON SCHEMA public TO creemcli_runtime; GRANT SELECT ON payment_storage_guard TO creemcli_runtime; GRANT SELECT,INSERT,UPDATE ON payment_orders,payment_events,payment_subscription_inbox TO creemcli_runtime; CREATE TABLE payment_subscription_upgrades(contract TEXT PRIMARY KEY,receipt BYTEA NOT NULL,committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()); REVOKE ALL ON payment_subscription_upgrades FROM PUBLIC; INSERT INTO payment_subscription_upgrades(contract,receipt) VALUES('original-epay-upgrade',convert_to('original-epay-receipt','UTF8')); INSERT INTO payment_subscription_inbox(evidence_id,order_id,event_id,event_digest,payload,signature) VALUES('pay_'||repeat('a',64),'sub_'||repeat('b',64),'old-event','old-digest',convert_to('original-epay-payload','UTF8'),convert_to('original-epay-signature','UTF8')); INSERT INTO payment_orders(id,namespace,version,snapshot) VALUES('original','epay/1/test',1,'{"id":"original","namespace":"epay/1/test"}');`)
	setup.Close()
	if err != nil {
		t.Fatal(err)
	}
	u.User = url.UserPassword("creemcli_installer", "private-fixture-only")
	credential := filepath.Join(t.TempDir(), "owner-url")
	if err = os.WriteFile(credential, []byte(u.String()), 0600); err != nil {
		t.Fatal(err)
	}
	args := []string{"--owner-url-file", credential, "--database", "lmm_payments_creem_cli_test", "--owner-role", "creemcli_owner", "--runtime-role", "creemcli_runtime"}
	var output bytes.Buffer
	if err = RunPaymentsCreemSubscriptionUpgradeCLI(context.Background(), args, &output); err == nil {
		t.Fatal("unfenced login runtime accepted")
	}
	if _, err = admin.Exec(`ALTER ROLE creemcli_runtime NOLOGIN`); err != nil {
		t.Fatal(err)
	}
	if err = RunPaymentsCreemSubscriptionUpgradeCLI(context.Background(), append(args, "--recover"), &output); err == nil {
		t.Fatal("recovery installed an absent upgrade")
	}
	output.Reset()
	if err = RunPaymentsCreemSubscriptionUpgradeCLI(context.Background(), args, &output); err != nil {
		t.Fatal(err)
	}
	var first payments.SubscriptionCreemUpgradeReceipt
	if err = json.Unmarshal(output.Bytes(), &first); err != nil || first.Status != "committed" || first.PreservedDataSHA256 == "" {
		t.Fatalf("missing commit receipt: %v", err)
	}
	output.Reset()
	if err = RunPaymentsCreemSubscriptionUpgradeCLI(context.Background(), append(args, "--recover"), &output); err != nil {
		t.Fatal(err)
	}
	var replay payments.SubscriptionCreemUpgradeReceipt
	if err = json.Unmarshal(output.Bytes(), &replay); err != nil || replay.Status != "already_committed" || replay.PreservedDataSHA256 != first.PreservedDataSHA256 || replay.PreservedACLSHA256 != first.PreservedACLSHA256 {
		t.Fatalf("original receipt changed: %v", err)
	}
	readback, err := sql.Open("postgres", u.String())
	if err != nil {
		t.Fatal(err)
	}
	defer readback.Close()
	if _, err = readback.Exec(`SET ROLE creemcli_owner`); err != nil {
		t.Fatal(err)
	}
	var intact bool
	err = readback.QueryRow(`SELECT
 (SELECT count(*)=1 FROM payment_orders) AND
 EXISTS(SELECT FROM payment_subscription_inbox WHERE event_id='old-event' AND payload=convert_to('original-epay-payload','UTF8') AND signature=convert_to('original-epay-signature','UTF8') AND state='pending') AND
 EXISTS(SELECT FROM payment_subscription_upgrades WHERE contract='original-epay-upgrade' AND receipt=convert_to('original-epay-receipt','UTF8')) AND
 NOT has_table_privilege('creemcli_runtime','payment_subscription_upgrades','INSERT,UPDATE,DELETE') AND
 EXISTS(SELECT FROM payment_orders WHERE id='original' AND version=1 AND snapshot='{"id":"original","namespace":"epay/1/test"}'::jsonb) AND
 has_table_privilege('creemcli_runtime','payment_creem_subscription_checkouts','SELECT') AND
 has_table_privilege('creemcli_runtime','payment_creem_subscription_checkouts','INSERT') AND
 has_table_privilege('creemcli_runtime','payment_creem_subscription_checkouts','UPDATE') AND
 NOT has_table_privilege('creemcli_runtime','payment_creem_subscription_checkouts','DELETE') AND
 has_table_privilege('creemcli_runtime','payment_creem_subscription_inbox','SELECT') AND
 has_table_privilege('creemcli_runtime','payment_creem_subscription_inbox','INSERT') AND
 has_table_privilege('creemcli_runtime','payment_creem_subscription_inbox','UPDATE') AND
 NOT has_table_privilege('creemcli_runtime','payment_creem_subscription_inbox','DELETE') AND
 has_table_privilege('creemcli_runtime','payment_subscription_inbox','SELECT') AND
 has_table_privilege('creemcli_runtime','payment_subscription_inbox','INSERT') AND
 has_table_privilege('creemcli_runtime','payment_subscription_inbox','UPDATE') AND
 NOT has_table_privilege('creemcli_runtime','payment_subscription_inbox','DELETE') AND
 NOT has_table_privilege('creemcli_runtime','payment_creem_subscription_upgrades','INSERT,UPDATE,DELETE')`).Scan(&intact)
	if err != nil || !intact {
		t.Fatalf("post-upgrade rows/privileges differ: %v", err)
	}
	t.Log("actual CLI rejected live runtime and absent recovery; owner upgrade and original receipt replay passed")
}
