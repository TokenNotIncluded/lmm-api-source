// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package freshinstall

import (
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/lib/pq"
)

// RunPaymentsSubscriptionUpgradeCLI is an explicit offline command. Runtime
// composition never loads owner credentials or installs this storage extension.
func RunPaymentsSubscriptionUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	var c Config
	var recoverOnly bool
	f := flag.NewFlagSet("upgrade-payments-subscriptions-db", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private installation login URL file")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact existing payments database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "NOLOGIN database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "fenced NOLOGIN runtime")
	f.BoolVar(&recoverOnly, "recover", false, "read the original committed upgrade without installing")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	c.Component = "payments"
	p, err := compiled(c.Component)
	if err != nil {
		return err
	}
	dsn, err := ownerURL(c, p)
	if err != nil {
		return err
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		return ErrRejected
	}
	defer db.Close()
	// One private connection keeps SET ROLE confined to this offline operation.
	// A replacement connection loses that role and is rejected by the upgrader.
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	ctx, cancel := context.WithTimeout(ctx, 40*time.Second)
	defer cancel()
	var allowed bool
	err = db.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND (SELECT count(*)=2 FROM pg_roles WHERE rolname IN($2,$3) AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname=$3))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,$2,'SET') OR pg_has_role(oid,$3,'SET')))
 FROM pg_database d WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
	if err != nil || !allowed {
		return ErrRejected
	}
	if _, err = db.ExecContext(ctx, "SET ROLE "+pq.QuoteIdentifier(c.OwnerRole)); err != nil {
		return ErrRejected
	}
	if recoverOnly {
		var exists bool
		if err = db.QueryRowContext(ctx, "SELECT to_regclass('public.payment_subscription_upgrades') IS NOT NULL").Scan(&exists); err != nil || !exists {
			return ErrRejected
		}
	}
	receipt, err := payments.UpgradeSubscriptionEPay(ctx, db, c.ExpectedDatabase, c.RuntimeRole)
	if err != nil {
		return err
	}
	if recoverOnly && receipt.Status != "already_committed" {
		return ErrCommitUnknown
	}
	if err = json.NewEncoder(out).Encode(receipt); err != nil {
		return ErrCommitUnknown
	}
	return nil
}
