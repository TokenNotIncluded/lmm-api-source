// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package walletupgrade

import (
	"context"
	"encoding/json"
	"flag"
	"io"
)

func RunCLI(ctx context.Context, args []string, out io.Writer) error {
	var c Config
	var recoverOnly bool
	f := flag.NewFlagSet("upgrade-wallet-db", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private offline installer Unix URL")
	f.StringVar(&c.Database, "database", "", "exact existing toolmarket database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "NOLOGIN database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "fenced NOLOGIN runtime role")
	f.StringVar(&c.OperationID, "operation-id", "", "stable original operation ID")
	f.BoolVar(&recoverOnly, "recover", false, "query original outcome without applying")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	var receipt Receipt
	var err error
	if recoverOnly {
		receipt, err = Recover(ctx, c)
	} else {
		receipt, err = Apply(ctx, c)
	}
	if err != nil {
		return err
	}
	if json.NewEncoder(out).Encode(receipt) != nil {
		return ErrUnknown
	}
	return nil
}
