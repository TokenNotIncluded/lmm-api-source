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
	return runPaymentsSubscriptionUpgradeCLI(ctx, args, out, "epay")
}

// RunPaymentsStripeSubscriptionUpgradeCLI installs only the additive Stripe
// successor to an already upgraded EPay database, under the same offline fence.
func RunPaymentsStripeSubscriptionUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	return runPaymentsSubscriptionUpgradeCLI(ctx, args, out, "stripe")
}

// RunPaymentsPancakeSubscriptionUpgradeCLI uses the same fenced owner connection.
func RunPaymentsPancakeSubscriptionUpgradeCLI(ctx context.Context, args []string, out io.Writer) error {
	return runPaymentsSubscriptionUpgradeCLI(ctx, args, out, "pancake")
}
func runPaymentsSubscriptionUpgradeCLI(ctx context.Context, args []string, out io.Writer, provider string) error {
	var c Config
	var recoverOnly bool
	command := "upgrade-payments-subscriptions-db"
	if provider == "stripe" {
		command = "upgrade-payments-stripe-subscriptions-db"
	}
	if provider == "pancake" {
		command = "upgrade-payments-pancake-subscriptions-db"
	}
	f := flag.NewFlagSet(command, flag.ContinueOnError)
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
		table := "public.payment_subscription_upgrades"
		if provider == "stripe" {
			table = "public.payment_stripe_subscription_upgrades"
		}
		if provider == "pancake" {
			table = "public.payment_pancake_subscription_upgrades"
		}
		if err = db.QueryRowContext(ctx, "SELECT to_regclass($1) IS NOT NULL", table).Scan(&exists); err != nil || !exists {
			return ErrRejected
		}
	}
	var receipt payments.SubscriptionEPayUpgradeReceipt
	if provider == "stripe" {
		var result payments.SubscriptionStripeUpgradeReceipt
		result, err = payments.UpgradeSubscriptionStripe(ctx, db, c.ExpectedDatabase, c.RuntimeRole)
		receipt = payments.SubscriptionEPayUpgradeReceipt(result)
	} else if provider == "pancake" {
		var result payments.SubscriptionPancakeUpgradeReceipt
		result, err = payments.UpgradeSubscriptionPancake(ctx, db, c.ExpectedDatabase, c.RuntimeRole)
		receipt = payments.SubscriptionEPayUpgradeReceipt(result)
	} else {
		receipt, err = payments.UpgradeSubscriptionEPay(ctx, db, c.ExpectedDatabase, c.RuntimeRole)
	}
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
