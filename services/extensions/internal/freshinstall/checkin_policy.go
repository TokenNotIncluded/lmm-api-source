package freshinstall

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/lib/pq"
	"io"
	"time"
)

// This explicit owner command uses only the extension DB. Normal startup never
// accepts an owner URL, schema mutation, or policy publication credential.
func RunCheckinPolicyCLI(parent context.Context, args []string, out io.Writer) error {
	var c Config
	var input string
	f := flag.NewFlagSet("publish-checkin-policy", flag.ContinueOnError)
	f.SetOutput(io.Discard)
	f.StringVar(&c.OwnerURLFile, "owner-url-file", "", "private offline owner connection")
	f.StringVar(&c.ExpectedDatabase, "database", "", "exact extension database")
	f.StringVar(&c.OwnerRole, "owner-role", "", "database owner")
	f.StringVar(&c.RuntimeRole, "runtime-role", "", "unprivileged runtime login")
	f.StringVar(&input, "input-file", "", "private policy command JSON")
	if f.Parse(args) != nil || f.NArg() != 0 {
		return ErrRejected
	}
	raw, e := coreclient.ReadPrivateFile(input, 16384)
	if e != nil {
		return ErrRejected
	}
	var command struct {
		OperationKey     string          `json:"operation_key"`
		ExpectedRevision int64           `json:"expected_revision"`
		Policy           checkins.Policy `json:"policy"`
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.DisallowUnknownFields()
	var extra any
	if d.Decode(&command) != nil || d.Decode(&extra) != io.EOF || command.Policy.Validate() != nil {
		return ErrRejected
	}
	dsn, e := ownerURL(c, plan{prefix: "lmm_checkins"})
	if e != nil {
		return ErrRejected
	}
	db, e := sql.Open("postgres", dsn)
	if e != nil {
		return ErrRejected
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(parent, 5*time.Second)
	defer cancel()
	var valid bool
	if db.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(datdba)=$2 AND current_user<>$3 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(current_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) FROM pg_database WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&valid) != nil || !valid {
		return ErrRejected
	}
	if _, e = db.ExecContext(ctx, "SET ROLE "+pq.QuoteIdentifier(c.OwnerRole)); e != nil {
		return ErrRejected
	}
	var version int
	if db.QueryRowContext(ctx, "SELECT version FROM lmm_checkins.contract WHERE singleton").Scan(&version) != nil || version != 1 {
		return ErrRejected
	}
	revision, e := checkins.PublishPolicy(ctx, db, command.OperationKey, command.ExpectedRevision, command.Policy)
	if e != nil {
		return e
	}
	return json.NewEncoder(out).Encode(map[string]any{"revision": revision, "database": c.ExpectedDatabase})
}
