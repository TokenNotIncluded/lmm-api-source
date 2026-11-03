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
	"strings"
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
	request := func(method, path, body string) *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest(method, path, strings.NewReader(body)))
		return w
	}
	for _, body := range []string{`{"days":0}`, `{"days":91}`} {
		if w := request("PUT", "/api/admin/acquisition/lookback", body); w.Code != 400 {
			t.Fatalf("invalid lookback accepted: %d", w.Code)
		}
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.accounts(user_id,lookback_days) VALUES(77,30)`); err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 2; i++ {
		if w := request("PUT", "/api/admin/acquisition/lookback", `{"days":7}`); w.Code != 200 {
			t.Fatalf("lookback failed: %d %s", w.Code, w.Body.String())
		}
	}
	var current, historical, policies int
	if err = db.QueryRow(`SELECT (SELECT lookback_days FROM lmm_acquisition.config),(SELECT lookback_days FROM lmm_acquisition.accounts WHERE user_id=77),(SELECT count(*) FROM lmm_acquisition.attribution_policies)`).Scan(&current, &historical, &policies); err != nil {
		t.Fatal(err)
	}
	if current != 7 || historical != 30 || policies != 2 {
		t.Fatalf("lookback rewrote history or duplicated policy: %d %d %d", current, historical, policies)
	}

	created := request("POST", "/api/admin/acquisition/links", `{"name":"Campaign","source":"original","target":"/pricing"}`)
	var saved struct {
		Data struct {
			ID     string
			Source string
			Name   string
		}
	}
	if err = json.Unmarshal(created.Body.Bytes(), &saved); err != nil || created.Code != 200 || saved.Data.ID == "" {
		t.Fatalf("create: %d %s", created.Code, created.Body.String())
	}
	id := saved.Data.ID
	changed := request("POST", "/api/admin/acquisition/links", `{"id":"`+id+`","name":"Renamed","source":"tampered","target":"/sign-in","archived":true}`)
	if err = json.Unmarshal(changed.Body.Bytes(), &saved); err != nil || changed.Code != 200 || saved.Data.Source != "original" || saved.Data.Name != "Renamed" {
		t.Fatalf("edit: %d %s", changed.Code, changed.Body.String())
	}
	removed := request("DELETE", "/api/admin/acquisition/links/"+id, "")
	if removed.Code != 200 {
		t.Fatalf("delete: %d %s", removed.Code, removed.Body.String())
	}
	var deletedAt int64
	if err = db.QueryRow(`SELECT deleted_at FROM lmm_acquisition.links WHERE id=$1`, id).Scan(&deletedAt); err != nil || deletedAt <= 0 {
		t.Fatalf("history removed: %v %d", err, deletedAt)
	}
	if w := request("POST", "/api/admin/acquisition/links", `{"id":"`+id+`","name":"Revive"}`); w.Code != 404 {
		t.Fatalf("deleted link updated: %d", w.Code)
	}
	if w := request("DELETE", "/api/admin/acquisition/links/"+id, ""); w.Code != 404 {
		t.Fatalf("deleted link deleted twice: %d", w.Code)
	}
	if w := request("POST", "/api/admin/acquisition/links", `{"name":"Bad","source":"source","target":"https://external.example"}`); w.Code != 400 {
		t.Fatalf("external target accepted: %d", w.Code)
	}

	denied := http.NewServeMux()
	s.Register(denied, listVerifier{0})
	w := httptest.NewRecorder()
	denied.ServeHTTP(w, httptest.NewRequest("GET", "/api/admin/acquisition/links", nil))
	for _, method := range []string{"POST", "DELETE"} {
		rejected := httptest.NewRecorder()
		path := "/api/admin/acquisition/links"
		if method == "DELETE" {
			path += "/" + id
		}
		denied.ServeHTTP(rejected, httptest.NewRequest(method, path, strings.NewReader(`{"name":"Forbidden"}`)))
		if rejected.Code != 403 {
			t.Fatalf("unprivileged write %s: %d", method, rejected.Code)
		}
	}
	if w.Code != 403 {
		t.Fatalf("unprivileged status %d", w.Code)
	}
}
