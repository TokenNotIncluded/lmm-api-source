// Package composition owns process resources, never core data or authority.
package composition

import (
	"context"
	"database/sql"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/postgresurl"
	"net/url"
	"regexp"
	"slices"
	"strings"
	"time"

	"github.com/lib/pq"
)

var databaseName = regexp.MustCompile(`^lmm_(store|support|assistant|payments|operations|toolmarket)(_[a-z0-9]+)*$`)

// StorageConfig names an independently provisioned extension database. Schema
// installation is explicit; startup never creates or migrates tables.
type StorageConfig struct {
	Component string
	URLFile   string
	Database  string
}

func (c StorageConfig) validate() error {
	if (c.Component != "store" && c.Component != "support" && c.Component != "assistant" && c.Component != "payments" && c.Component != "operations" && c.Component != "toolmarket") || c.URLFile == "" ||
		!databaseName.MatchString(c.Database) || !strings.HasPrefix(c.Database, "lmm_"+c.Component) {
		return errors.New("invalid dedicated extension storage configuration")
	}
	return nil
}

func storageURL(c StorageConfig) (string, error) {
	if err := c.validate(); err != nil {
		return "", err
	}
	b, err := coreclient.ReadPrivateFile(c.URLFile, 8192)
	if err != nil {
		return "", errors.New("cannot read bounded extension storage credential")
	}
	dsn := strings.TrimSpace(string(b))
	u, err := url.Parse(dsn)
	if err != nil || (u.Scheme != "postgres" && u.Scheme != "postgresql") || u.User == nil || u.User.Username() == "" ||
		u.Fragment != "" || strings.ContainsAny(dsn, "\r\n\x00") {
		return "", errors.New("invalid extension PostgreSQL URL")
	}
	unix, err := postgresurl.Unix(u)
	if err != nil {
		return "", errors.New("invalid extension Unix PostgreSQL URL or connection environment")
	}
	name, err := url.PathUnescape(strings.TrimPrefix(u.EscapedPath(), "/"))
	if err != nil || name != c.Database {
		return "", errors.New("extension PostgreSQL database does not match its component")
	}
	if unix {
		return dsn, nil
	}
	// URL query parameters cannot replace the checked database, role or host, or
	// install a caller-controlled search_path/statement configuration.
	if _, err := postgresurl.TCP(u); err != nil {
		return "", errors.New("invalid extension PostgreSQL TCP transport policy")
	}
	for key, values := range u.Query() {
		if len(values) != 1 || (key != "sslmode" && key != "sslrootcert" && key != "connect_timeout" && key != "application_name") {
			return "", errors.New("unsupported extension PostgreSQL URL option")
		}
	}
	return dsn, nil
}

// openStorage is the single production pool-opening boundary. Modules receive
// only their own pool. A runtime role must not administer PostgreSQL or connect
// to a core database; a dedicated database must not contain another module.
func openStorage(ctx context.Context, c StorageConfig) (*sql.DB, error) {
	return openStorageBundle(ctx, c, defaultStorageBundle)
}

func openStorageBundle(ctx context.Context, c StorageConfig, bundle storageBundle) (*sql.DB, error) {
	policy, err := storagePolicyFor(c.Component, bundle)
	if err != nil {
		return nil, err
	}
	dsn, err := storageURL(c)
	if err != nil {
		return nil, err
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		return nil, errors.New("extension storage unavailable")
	}
	db.SetMaxOpenConns(2)
	db.SetMaxIdleConns(2)
	db.SetConnMaxIdleTime(5 * time.Minute)
	failed := true
	defer func() {
		if failed {
			_ = db.Close()
		}
	}()
	bounded, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var name string
	var privileged, foreignSchema, coreConnect, rolePaths bool
	err = db.QueryRowContext(bounded, `SELECT current_database(),
	 EXISTS(SELECT 1 FROM pg_roles WHERE rolname=current_user AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) OR has_database_privilege(current_user,current_database(),'CREATE') OR has_database_privilege(current_user,current_database(),'TEMP') OR EXISTS(SELECT 1 FROM pg_namespace WHERE nspname NOT LIKE 'pg\_%' ESCAPE '\' AND nspname<>'information_schema' AND has_schema_privilege(current_user,oid,'CREATE')),
	 EXISTS(SELECT 1 FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\' OR ((nspname LIKE 'lmm\_%' ESCAPE '\' OR nspname='extension_wallet' OR ($3 AND nspname NOT LIKE 'pg\_%' ESCAPE '\' AND nspname NOT IN ('public','information_schema'))) AND NOT(nspname=ANY($1::text[])))),
	 EXISTS(SELECT 1 FROM pg_database WHERE (datname LIKE '%core%' OR (datname LIKE 'lmm\_%' ESCAPE '\' AND datname <> $2)) AND has_database_privilege(current_user,oid,'CONNECT')),
	 EXISTS(SELECT 1 FROM pg_roles WHERE rolname<>current_user AND (pg_has_role(current_user,oid,'SET') OR pg_has_role(current_user,oid,'USAGE') OR pg_has_role(current_user,oid,'MEMBER')))`, pq.Array(policy.namespaces), c.Database, c.Component == "toolmarket").Scan(&name, &privileged, &foreignSchema, &coreConnect, &rolePaths)
	if err != nil {
		return nil, errors.New("extension storage guard unavailable")
	}
	if name != c.Database || privileged || foreignSchema || coreConnect || rolePaths {
		return nil, errors.New("extension storage isolation rejected")
	}
	if c.Component == "assistant" {
		if err := checkAssistantAuthority(bounded, db); err != nil {
			return nil, err
		}
	}
	limits := policy
	if c.Component == "assistant" {
		// Fresh installs also contain optional capabilities. They need not exist
		// or have minimum grants when disabled, but must never exceed their caps.
		limits, _ = storagePolicyFor("assistant", assistantAgentBundle)
		names := make([]string, 0, len(policy.tables))
		for _, table := range policy.tables {
			names = append(names, table.name)
		}
		var valid bool
		if err := db.QueryRowContext(bounded, `SELECT NOT EXISTS(SELECT FROM unnest($1::text[]) AS t(name) WHERE NOT EXISTS(SELECT FROM pg_class WHERE oid=to_regclass(t.name) AND relkind IN ('r','p')))`, pq.Array(names)).Scan(&valid); err != nil || !valid {
			return nil, errors.New("extension storage table unavailable")
		}
	}
	// Keep each privilege separate (PostgreSQL comma lists mean OR), while
	// batching checks to fit the bounded startup even over a TCP connection.
	for _, check := range []struct {
		tables   []storageTable
		function string
		want     bool
	}{
		{policy.tables, "has_table_privilege", true},
		{limits.forbidden, "has_table_privilege", false},
		{limits.forbiddenColumns, "has_any_column_privilege", false},
	} {
		var names, privileges []string
		for _, table := range check.tables {
			for _, privilege := range table.privileges {
				names, privileges = append(names, table.name), append(privileges, privilege)
			}
		}
		if len(names) == 0 {
			continue
		}
		var valid bool
		query := `SELECT NOT EXISTS(SELECT FROM unnest($1::text[],$2::text[]) AS t(name,privilege) WHERE COALESCE(` + check.function + `(current_user,to_regclass(t.name),t.privilege),false)<>$3)`
		if err := db.QueryRowContext(bounded, query, pq.Array(names), pq.Array(privileges), check.want).Scan(&valid); err != nil || !valid {
			return nil, errors.New("extension storage grants differ from runtime contract")
		}
	}
	if c.Component == "assistant" || bundle == toolMarketBuiltinsBundle {
		for _, namespace := range policy.namespaces {
			var allowed bool
			if err := db.QueryRowContext(bounded, `SELECT COALESCE(has_schema_privilege(current_user,$1,'USAGE'),false)`, namespace).Scan(&allowed); err != nil || !allowed {
				return nil, errors.New("extension bundled storage schema grants unavailable")
			}
		}
	}
	for _, contract := range policy.contracts {
		var valid bool
		// The relation names and versions are compiled policy, never configuration.
		if err := db.QueryRowContext(bounded, `SELECT count(*)=1 AND COALESCE(bool_and(singleton AND version=$1),false) FROM `+contract.name, contract.version).Scan(&valid); err != nil || !valid {
			return nil, errors.New("extension storage contract version unavailable")
		}
	}
	for _, grant := range limits.updateColumns {
		var actual []string
		// has_column_privilege includes table UPDATE. Inspect every real column,
		// not just named grants: a table grant must not hide additional authority.
		if err := db.QueryRowContext(bounded, `SELECT COALESCE(array_agg(attname::text ORDER BY attname::text) FILTER (WHERE has_column_privilege(current_user,attrelid,attnum,'UPDATE')),ARRAY[]::text[]) FROM pg_attribute WHERE attrelid=to_regclass($1) AND attnum>0 AND NOT attisdropped`, grant.name).Scan(pq.Array(&actual)); err != nil {
			return nil, errors.New("extension storage column grants unavailable")
		}
		want := slices.Clone(grant.columns)
		slices.Sort(want)
		slices.Sort(actual)
		required := slices.ContainsFunc(policy.updateColumns, func(g storageColumnGrant) bool { return g.name == grant.name })
		extra := slices.ContainsFunc(actual, func(column string) bool { return !slices.Contains(want, column) })
		if extra || (required && !slices.Equal(actual, want)) {
			return nil, errors.New("extension storage UPDATE columns differ from runtime contract")
		}
	}
	for _, seq := range limits.sequences {
		var allowed, excessive bool
		if err := db.QueryRowContext(bounded, `SELECT COALESCE(has_sequence_privilege(current_user,to_regclass($1),'USAGE'),false),COALESCE(has_sequence_privilege(current_user,to_regclass($1),'SELECT,UPDATE'),false)`, seq).Scan(&allowed, &excessive); err != nil || excessive || (!allowed && slices.Contains(policy.sequences, seq)) {
			return nil, errors.New("assistant sequence grants differ from runtime contract")
		}
	}
	if c.Component == "operations" {
		for _, privilege := range []string{"USAGE", "SELECT"} {
			var allowed bool
			if err := db.QueryRowContext(bounded, `SELECT COALESCE(has_sequence_privilege(current_user,to_regclass('lmm_operations.publication_id'),$1),false)`, privilege).Scan(&allowed); err != nil || !allowed {
				return nil, errors.New("operations sequence grants unavailable")
			}
		}
	}
	failed = false
	return db, nil
}

// Membership paths are rejected before this check. Inspect ACLs themselves as
// effective privilege checks cannot distinguish PUBLIC or WITH GRANT OPTION.
// Check optional installed objects too, without querying their journal rows or
// treating their presence as permission to compose AgentChat.
func checkAssistantAuthority(ctx context.Context, db *sql.DB) error {
	var unsafe bool
	err := db.QueryRowContext(ctx, `WITH runtime AS (SELECT oid FROM pg_roles WHERE rolname=current_user),
	 schemas AS (SELECT oid,nspowner,nspacl FROM pg_namespace WHERE nspname='lmm_assistant'),
	 relations AS (SELECT c.oid,c.relowner,c.relacl FROM pg_class c JOIN schemas n ON n.oid=c.relnamespace),
	 acls AS (
	   SELECT a.* FROM schemas n CROSS JOIN LATERAL aclexplode(n.nspacl) a
	   UNION ALL SELECT a.* FROM relations c CROSS JOIN LATERAL aclexplode(c.relacl) a
	   UNION ALL SELECT a.* FROM pg_attribute c JOIN relations r ON r.oid=c.attrelid CROSS JOIN LATERAL aclexplode(c.attacl) a WHERE c.attnum>0 AND NOT c.attisdropped
	 ) SELECT EXISTS(SELECT FROM pg_database d,runtime r WHERE d.datname=current_database() AND d.datdba=r.oid)
	 OR EXISTS(SELECT FROM pg_namespace n,runtime r WHERE n.nspowner=r.oid)
	 OR EXISTS(SELECT FROM relations c,runtime r WHERE c.relowner=r.oid)
	 OR EXISTS(SELECT FROM acls a,runtime r WHERE a.grantee=0 OR (a.grantee=r.oid AND a.is_grantable))
	 OR EXISTS(SELECT FROM pg_database d CROSS JOIN LATERAL aclexplode(d.datacl) a,runtime r WHERE d.datname=current_database() AND a.grantee=r.oid AND a.is_grantable)`).Scan(&unsafe)
	if err != nil || unsafe {
		return errors.New("assistant storage ownership or ACL exceeds runtime authority")
	}
	return nil
}

type storageTable struct {
	name       string
	privileges []string
}

func storageTables(component string) []storageTable {
	var tables []storageTable
	add := func(name string, privs ...string) { tables = append(tables, storageTable{name, privs}) }
	switch component {
	case "toolmarket":
		add("lmm_toolmarket.snapshot", "SELECT", "UPDATE")
	case "store":
		for _, name := range []string{"shops", "products", "variants", "orders", "commands", "stock_movements", "money_operations"} {
			add("lmm_store."+name, "SELECT", "INSERT", "UPDATE")
		}
	case "support":
		for _, name := range []string{"customers", "conversations", "messages", "commands"} {
			add("lmm_support."+name, "SELECT", "INSERT", "UPDATE")
		}
	case "assistant":
		add("lmm_assistant.contract", "SELECT")
		add("lmm_assistant.records", "SELECT", "INSERT", "UPDATE")
		add("lmm_assistant.conversations", "SELECT", "INSERT", "UPDATE")
		add("lmm_assistant.history_messages", "SELECT", "INSERT", "DELETE")
		add("lmm_assistant.turn_receipts", "SELECT", "INSERT")
		add("lmm_assistant.preset_stats", "SELECT")
	case "operations":
		add("lmm_operations.contract", "SELECT")
		add("lmm_operations.publications", "SELECT", "INSERT")
		add("lmm_operations.reads", "SELECT", "INSERT")
	case "payments":
		add("public.payment_storage_guard", "SELECT")
		add("public.payment_orders", "SELECT", "INSERT", "UPDATE")
		add("public.payment_events", "SELECT", "INSERT", "UPDATE")
	}
	return tables
}
