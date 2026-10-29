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
	if len(os.Args) > 1 && os.Args[1] == "upgrade-wallet-db" {
		if err := walletupgrade.RunCLI(context.Background(), os.Args[2:], os.Stdout); err != nil {
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
