package acquisition

import (
	"context"
	"database/sql"
	"encoding/json"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	_ "github.com/lib/pq"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
)

type listVerifier struct{ level int32 }

func (v listVerifier) Verify(context.Context, *http.Request) (*pb.AuthorizeResponse, error) {
	return &pb.AuthorizeResponse{UserId: 1, PlatformLevel: v.level}, nil
}
func TestLinksPostgresHTTP(t *testing.T) {
	dsn := os.Getenv("ACQUISITION_TEST_DATABASE_URL")
	if dsn == "" {
		t.Skip("requires isolated PostgreSQL database")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var exists bool
	if err = db.QueryRow(`SELECT to_regnamespace('lmm_acquisition') IS NOT NULL`).Scan(&exists); err != nil {
		t.Fatal(err)
	}
	if exists {
		t.Fatal("requires empty database")
	}
	if _, err = db.Exec(Schema); err != nil {
		t.Fatal(err)
	}
	defer db.Exec(`DROP SCHEMA lmm_acquisition CASCADE`)
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.links(id,name,archived,created_at,deleted_at) VALUES(repeat('a',32),'100% literal',false,3,0),(repeat('b',32),'100X literal',true,2,0),(repeat('c',32),'deleted',false,1,1)`); err != nil {
		t.Fatal(err)
	}
	s, err := New(context.Background(), db, "https://synthetic.example", true)
	if err != nil {
		t.Fatal(err)
	}
	mux := http.NewServeMux()
	s.Register(mux, listVerifier{5})
	for _, c := range []struct {
		query        string
		total, items int
	}{{"", 2, 2}, {"?status=active", 1, 1}, {"?status=archived", 1, 1}, {"?status=deleted", 1, 1}, {"?q=100%25", 1, 1}, {"?page_size=1&page=2", 2, 1}, {"?page=99", 2, 0}} {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", "/api/admin/acquisition/links"+c.query, nil))
		var body struct {
			Success bool
			Data    struct {
				Total int
				Items []json.RawMessage
			}
		}
		if err = json.Unmarshal(w.Body.Bytes(), &body); err != nil {
			t.Fatal(err)
		}
		if w.Code != 200 || !body.Success || body.Data.Total != c.total || len(body.Data.Items) != c.items {
			t.Fatalf("%s: %d %s", c.query, w.Code, w.Body.String())
		}
	}
	for _, q := range []string{"?status=bad", "?page_size=101", "?page=100001"} {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", "/api/admin/acquisition/links"+q, nil))
		if w.Code != 400 {
			t.Fatalf("invalid query accepted: %s %d", q, w.Code)
		}
	}
	denied := http.NewServeMux()
	s.Register(denied, listVerifier{0})
	w := httptest.NewRecorder()
	denied.ServeHTTP(w, httptest.NewRequest("GET", "/api/admin/acquisition/links", nil))
	if w.Code != 403 {
		t.Fatalf("unprivileged status %d", w.Code)
	}
}
