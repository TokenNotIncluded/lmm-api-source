package main

import (
	"context"
	"log"
	"os"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/app"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/walletupgrade"
)

func main() {
	if len(os.Args) > 1 && os.Args[1] == "publish-checkin-policy" {
		if err := freshinstall.RunCheckinPolicyCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-script-repository-credentials" {
		if err := freshinstall.RunScriptRepositoryCredentialsUpgradeCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-payments-pancake-subscriptions-db" {
		if err := freshinstall.RunPaymentsPancakeSubscriptionUpgradeCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-payments-stripe-subscriptions-db" {
		if err := freshinstall.RunPaymentsStripeSubscriptionUpgradeCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-payments-subscriptions-db" {
		if err := freshinstall.RunPaymentsSubscriptionUpgradeCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-operations-db" {
		if err := freshinstall.RunOperationsUpgradeCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-wallet-db" {
		if err := walletupgrade.RunCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "upgrade-ionet" {
		if err := freshinstall.RunIoNetUpgradeCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "install-db" {
		if err := freshinstall.RunCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
			log.Fatal(err)
		}
		return
	}
	if err := app.Run(); err != nil {
		log.Fatal(err)
	}
}
