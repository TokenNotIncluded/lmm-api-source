package freshinstall

import (
	"context"
	"database/sql"
	"fmt"
	"github.com/lib/pq"
	"os"
	"strings"
	"testing"
	"time"
)

func TestAcquisitionCompiledSchemaAndRuntimeACL(t *testing.T) {
	dsn := os.Getenv("ACQUISITION_TEST_DATABASE_URL")
	if dsn == "" {
		t.Skip("requires isolated PostgreSQL database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	tx, err := db.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer tx.Rollback()
	role := fmt.Sprintf("acquisition_acl_%d", time.Now().UnixNano())
	if _, err = tx.Exec("CREATE ROLE " + pq.QuoteIdentifier(role) + " NOLOGIN NOSUPERUSER"); err != nil {
		t.Fatal(err)
	}
	p, err := compiled("acquisition")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = tx.Exec(p.schema); err != nil {
		t.Fatal(err)
	}
	for _, grant := range p.grants {
		if _, err = tx.Exec(grant + pq.QuoteIdentifier(role)); err != nil {
			t.Fatal(err)
		}
	}
	if _, err = tx.Exec("SET LOCAL ROLE " + pq.QuoteIdentifier(role)); err != nil {
		t.Fatal(err)
	}
	var version int
	if err = tx.QueryRow(`SELECT version FROM lmm_acquisition.contract`).Scan(&version); err != nil || version != 1 {
		t.Fatalf("contract: %v %d", err, version)
	}
	if _, err = tx.Exec(`INSERT INTO lmm_acquisition.links(id,name,source,target) VALUES(repeat('a',32),'Synthetic','source','/pricing')`); err != nil {
		t.Fatal(err)
	}
	if _, err = tx.Exec(`INSERT INTO lmm_acquisition.corrections(user_id,previous_revision,previous_source,source,reason,actor_id,created_at) VALUES(1,0,'unknown','manual','synthetic',1,1)`); err != nil {
		t.Fatal(err)
	}
	var canMutate bool
	if err = tx.QueryRow(`SELECT has_table_privilege('lmm_acquisition.contract','INSERT,UPDATE,DELETE,TRUNCATE') OR has_table_privilege('lmm_acquisition.corrections','UPDATE,DELETE,TRUNCATE')`).Scan(&canMutate); err != nil || canMutate {
		t.Fatalf("authority expanded: %v %v", err, canMutate)
	}
	if !strings.Contains(p.prefix, "acquisition") {
		t.Fatal("wrong database prefix")
	}
}
