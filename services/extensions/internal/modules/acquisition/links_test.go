package acquisition

import (
	"context"
	"database/sql"
	"encoding/csv"
	"encoding/json"
	"fmt"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
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

type testUserRead struct{ role string }

func (v testUserRead) Users(ctx context.Context, ids []int64) (map[int64]community.CoreUserInfo, error) {
	return map[int64]community.CoreUserInfo{77: {UserID: 77, Active: true, PlatformRole: v.role}}, nil
}
func (v testUserRead) UserPage(ctx context.Context, after int64, limit int) (community.RegistrationPage, error) {
	return community.RegistrationPage{Users: []community.RegistrationUser{{UserID: 77, PlatformRole: v.role, Active: true}}, NextID: 77}, nil
}
func (v testUserRead) NativeActivity(ctx context.Context, ids []int64, from, to int64) (community.NativeActivity, error) {
	at := from + 1
	return community.NativeActivity{NativeOnly: true, Rows: []community.ActivityDay{{UserID: 77, Day: (at / daySeconds) * daySeconds, FirstAt: at, LastAt: at}}}, nil
}

type revokeActivityRead struct {
	testUserRead
	s *Service
}

func (v revokeActivityRead) NativeActivity(ctx context.Context, ids []int64, from, to int64) (community.NativeActivity, error) {
	if err := v.s.RevokeAccount(ctx, 77); err != nil {
		return community.NativeActivity{}, err
	}
	return v.testUserRead.NativeActivity(ctx, ids, from, to)
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
	now := s.now().Unix()
	for _, amount := range []int{1234567, 7654321} {
		body := fmt.Sprintf(`{"link_id":%q,"from":%d,"to":%d,"observation_days":7,"currency":"USD","amount_micros":%d}`, id, now-100, now-1, amount)
		if w := request("PUT", "/api/admin/acquisition/cost", body); w.Code != 200 {
			t.Fatalf("cost write: %d %s", w.Code, w.Body.String())
		}
	}
	var costCount int
	var amount int64
	if err = db.QueryRow(`SELECT count(*),max(amount_micros) FROM lmm_acquisition.costs`).Scan(&costCount, &amount); err != nil || costCount != 1 || amount != 7654321 {
		t.Fatalf("cost upsert: %v %d %d", err, costCount, amount)
	}
	if w := request("PUT", "/api/admin/acquisition/cost", `{"amount_micros":-1}`); w.Code != 400 {
		t.Fatalf("invalid cost accepted %d", w.Code)
	}

	changed := request("POST", "/api/admin/acquisition/links", `{"id":"`+id+`","name":"Renamed","source":"tampered","target":"/sign-in","archived":true}`)
	if err = json.Unmarshal(changed.Body.Bytes(), &saved); err != nil || changed.Code != 200 || saved.Data.Source != "original" || saved.Data.Name != "Renamed" {
		t.Fatalf("edit: %d %s", changed.Code, changed.Body.String())
	}
	preview := request("GET", "/api/admin/acquisition/links/"+id+"/preview", "")
	var view struct {
		Data struct {
			LinkID   string `json:"link_id"`
			Source   string
			Target   string
			Evidence string
			Excluded bool
			Archived bool
		}
	}
	if err = json.Unmarshal(preview.Body.Bytes(), &view); err != nil || preview.Code != 200 || view.Data.LinkID != id || view.Data.Source != "original" || view.Data.Target != "/pricing" || view.Data.Evidence != "promotion_link" || !view.Data.Excluded || !view.Data.Archived {
		t.Fatalf("preview: %d %s", preview.Code, preview.Body.String())
	}
	if len(preview.Result().Cookies()) != 0 {
		t.Fatal("preview created tracking cookie")
	}
	var observations int
	if err = db.QueryRow(`SELECT (SELECT count(*) FROM lmm_acquisition.visits)+(SELECT count(*) FROM lmm_acquisition.visitors)+(SELECT count(*) FROM lmm_acquisition.consents)`).Scan(&observations); err != nil || observations != 0 {
		t.Fatalf("preview tracked visit: %v %d", err, observations)
	}

	removed := request("DELETE", "/api/admin/acquisition/links/"+id, "")
	if removed.Code != 200 {
		t.Fatalf("delete: %d %s", removed.Code, removed.Body.String())
	}
	if w := request("GET", "/api/admin/acquisition/links/"+id+"/preview", ""); w.Code != 404 {
		t.Fatalf("deleted preview visible: %d", w.Code)
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

	if _, err = db.Exec(`UPDATE lmm_acquisition.config SET started_at=$1;`, now-20); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.visitors(id,user_id,created_at) VALUES(repeat('d',64),0,$1),(repeat('e',64),77,$1),(repeat('f',64),88,$1);`, now-10); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.visits(visitor_id,nonce,source,created_at) VALUES(repeat('d',64),'a','alpha',$1),(repeat('d',64),'b','beta',$1),(repeat('e',64),'a','alpha',$1),(repeat('f',64),'a','alpha',$1),(repeat('e',64),'end','excluded',$2)`, now-10, now); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		role   string
		count  int
		status int
	}{{"user", 2, 200}, {"admin", 1, 200}, {"", 0, 503}} {
		s.UserRead = testUserRead{tc.role}
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/visitors?from=%d&to=%d", now-30, now), nil))
		if w.Code != tc.status {
			t.Fatalf("visitor status role=%q: %d %s", tc.role, w.Code, w.Body.String())
		}
		if tc.status != 200 {
			continue
		}
		var summary struct {
			Data struct {
				Count     int   `json:"observed_visitors"`
				Available int64 `json:"available_from"`
				Complete  bool  `json:"coverage_complete"`
				Channels  []struct {
					Source   string
					Visitors int
				}
			}
		}
		if err = json.Unmarshal(w.Body.Bytes(), &summary); err != nil || summary.Data.Count != tc.count || summary.Data.Available != now-20 || summary.Data.Complete || len(summary.Data.Channels) != 2 || summary.Data.Channels[0].Source != "alpha" || summary.Data.Channels[0].Visitors != tc.count {
			t.Fatalf("visitor summary %v: %s", err, w.Body.String())
		}
	}

	rootMux := http.NewServeMux()
	s.Register(rootMux, listVerifier{6})
	s.UserRead = testUserRead{"user"}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.corrections(user_id,previous_revision,previous_source,source,reason,actor_id,created_at) SELECT 77,n,'old','corrected','synthetic',1,$1 FROM generate_series(1,101) n`, s.now().Unix()); err != nil {
		t.Fatal(err)
	}
	detail := func() *httptest.ResponseRecorder {
		w := httptest.NewRecorder()
		rootMux.ServeHTTP(w, httptest.NewRequest("GET", "/api/admin/acquisition/users/77", nil))
		return w
	}
	unknown := detail()
	var info struct {
		Data struct {
			RegisteredAt *int64 `json:"registered_at"`
			Historical   *bool  `json:"historical"`
			Recent       []json.RawMessage
			RecentLimit  int   `json:"recent_limit"`
			FirstSuccess int64 `json:"first_success_at"`
		}
	}
	if err = json.Unmarshal(unknown.Body.Bytes(), &info); err != nil || unknown.Code != 200 || info.Data.RegisteredAt != nil || info.Data.Historical != nil {
		t.Fatalf("unknown registration: %d %s", unknown.Code, unknown.Body.String())
	}
	unknownList := httptest.NewRecorder()
	rootMux.ServeHTTP(unknownList, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/users?from=%d&to=%d", now-200, now), nil))
	var coverage struct {
		Data struct {
			Unknown  int  `json:"unknown_registration_count"`
			Complete bool `json:"coverage_complete"`
			Total    int
		}
	}
	if err = json.Unmarshal(unknownList.Body.Bytes(), &coverage); err != nil || unknownList.Code != 200 || coverage.Data.Unknown != 1 || coverage.Data.Complete || coverage.Data.Total != 0 {
		t.Fatalf("unknown list coverage: %s", unknownList.Body.String())
	}

	incompleteExport := httptest.NewRecorder()
	rootMux.ServeHTTP(incompleteExport, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/users/export?from=%d&to=%d", now-200, now), nil))
	if incompleteExport.Code != 503 {
		t.Fatalf("incomplete cohort export: %d", incompleteExport.Code)
	}

	if _, err = db.Exec(`INSERT INTO lmm_acquisition.cohort(user_id,registered_at,origin) VALUES(77,$1,'legacy_import')`, now-100); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.activities(user_id,day,first_at,last_at) VALUES(77,0,$1,$1)`, now-50); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.visits(visitor_id,nonce,source,created_at) SELECT repeat('e',64),'detail-'||n,'alpha',$1-n FROM generate_series(0,24) n`, now); err != nil {
		t.Fatal(err)
	}
	known := detail()
	if err = json.Unmarshal(known.Body.Bytes(), &info); err != nil || known.Code != 200 || info.Data.RegisteredAt == nil || *info.Data.RegisteredAt != now-100 || info.Data.Historical == nil || !*info.Data.Historical || len(info.Data.Recent) != 20 || info.Data.RecentLimit != 20 || info.Data.FirstSuccess != now-50 {
		t.Fatalf("known detail: %d %s", known.Code, known.Body.String())
	}
	if strings.Contains(known.Body.String(), "visitor_id") || strings.Contains(known.Body.String(), "nonce") {
		t.Fatal("detail exposed browser tracking identifiers")
	}

	list := httptest.NewRecorder()
	rootMux.ServeHTTP(list, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/users?from=%d&to=%d", now-200, now), nil))
	var listing struct {
		Data struct {
			Items []struct {
				UserID int64 `json:"user_id"`
				Source string
			}
			Total    int
			Complete bool `json:"coverage_complete"`
		}
	}
	if err = json.Unmarshal(list.Body.Bytes(), &listing); err != nil || list.Code != 200 || listing.Data.Total != 1 || len(listing.Data.Items) != 1 || listing.Data.Items[0].UserID != 77 || listing.Data.Items[0].Source != "historical_unrecorded" || !listing.Data.Complete {
		t.Fatalf("user listing %d: %s", list.Code, list.Body.String())
	}

	for _, query := range []string{"&source=not-observed", "&page=2"} {
		w := httptest.NewRecorder()
		rootMux.ServeHTTP(w, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/users?from=%d&to=%d%s", now-200, now, query), nil))
		var p struct {
			Data struct{ Items []json.RawMessage }
		}
		if err = json.Unmarshal(w.Body.Bytes(), &p); err != nil || w.Code != 200 || len(p.Data.Items) != 0 {
			t.Fatalf("filtered user list %q: %d %s", query, w.Code, w.Body.String())
		}
	}

	history := httptest.NewRecorder()
	rootMux.ServeHTTP(history, httptest.NewRequest("GET", "/api/admin/acquisition/users/77/corrections", nil))
	var changes struct {
		Data struct {
			Items   []json.RawMessage
			HasMore bool `json:"has_more"`
		}
	}
	if err = json.Unmarshal(history.Body.Bytes(), &changes); err != nil || history.Code != 200 || len(changes.Data.Items) != 100 || !changes.Data.HasMore {
		t.Fatalf("correction history: %d %s", history.Code, history.Body.String())
	}
	save := func(body string) int {
		w := httptest.NewRecorder()
		rootMux.ServeHTTP(w, httptest.NewRequest("POST", "/api/admin/acquisition/users/77/corrections", strings.NewReader(body)))
		return w.Code
	}
	if status := save(`{"expected_revision":0,"source":"manual","reason":"operator correction"}`); status != 200 {
		t.Fatalf("save correction: %d", status)
	}
	if status := save(`{"expected_revision":0,"source":"stale","reason":"operator correction"}`); status != 409 {
		t.Fatalf("stale revision: %d", status)
	}
	var revision int64
	var source string
	if err = db.QueryRow(`SELECT revision,source FROM lmm_acquisition.correction_heads WHERE user_id=77`).Scan(&revision, &source); err != nil || source != "manual" {
		t.Fatalf("head %d %q %v", revision, source, err)
	}
	start := make(chan struct{})
	results := make(chan int, 2)
	body := fmt.Sprintf(`{"expected_revision":%d,"source":"parallel","reason":"concurrent operator correction"}`, revision)
	for n := 0; n < 2; n++ {
		go func() { <-start; results <- save(body) }()
	}
	close(start)
	first, second := <-results, <-results
	if !((first == 200 && second == 409) || (first == 409 && second == 200)) {
		t.Fatalf("concurrent correction: %d %d", first, second)
	}
	var appended int
	if err = db.QueryRow(`SELECT count(*) FROM lmm_acquisition.corrections WHERE user_id=77 AND previous_revision=$1`, revision).Scan(&appended); err != nil || appended != 1 {
		t.Fatalf("concurrent audit count %d: %v", appended, err)
	}
	if err = db.QueryRow(`SELECT revision,source FROM lmm_acquisition.correction_heads WHERE user_id=77`).Scan(&revision, &source); err != nil || source != "parallel" {
		t.Fatalf("concurrent head %d %q: %v", revision, source, err)
	}

	formula := " \u200b=1+1,中文"
	if _, err = db.Exec(`UPDATE lmm_acquisition.correction_heads SET source=$1 WHERE user_id=77`, formula); err != nil {
		t.Fatal(err)
	}
	export := httptest.NewRecorder()
	rootMux.ServeHTTP(export, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/users/export?from=%d&to=%d", now-200, now), nil))
	records, csvErr := csv.NewReader(strings.NewReader(export.Body.String())).ReadAll()
	if export.Code != 200 || csvErr != nil || len(records) != 2 || len(records[1]) != 10 || records[1][0] != "77" || records[1][4] != "'"+formula || export.Header().Get("Content-Type") != "text/csv; charset=utf-8" || !strings.Contains(export.Header().Get("Cache-Control"), "no-store") {
		t.Fatalf("CSV export: %d %v %s", export.Code, csvErr, export.Body.String())
	}
	deniedExport := httptest.NewRecorder()
	mux.ServeHTTP(deniedExport, httptest.NewRequest("GET", fmt.Sprintf("/api/admin/acquisition/users/export?from=%d&to=%d", now-200, now), nil))
	if deniedExport.Code != 403 {
		t.Fatalf("admin CSV permission: %d", deniedExport.Code)
	}

	if _, err = db.Exec(`UPDATE lmm_acquisition.config SET started_at=$1`, now-200); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`UPDATE lmm_acquisition.accounts SET consent_version=2,created_at=$1 WHERE user_id=77`, now); err != nil {
		t.Fatal(err)
	}
	if _, err = db.Exec(`INSERT INTO lmm_acquisition.consents(user_id,allowed,version,updated_at) VALUES(77,true,2,$1)`, now); err != nil {
		t.Fatal(err)
	}
	rebuild := func() {
		w := httptest.NewRecorder()
		rootMux.ServeHTTP(w, httptest.NewRequest("POST", "/api/admin/acquisition/activity/rebuild", nil))
		if w.Code != 200 || !strings.Contains(w.Body.String(), `"incomplete":true`) {
			t.Fatalf("native rebuild: %d %s", w.Code, w.Body.String())
		}
	}
	rebuild()
	rebuild()
	var nativeRows int
	var incomplete bool
	if err = db.QueryRow(`SELECT count(*) FROM lmm_acquisition.activities WHERE user_id=77 AND day>0`).Scan(&nativeRows); err != nil || nativeRows != 1 {
		t.Fatalf("idempotent daily activity %d %v", nativeRows, err)
	}
	if err = db.QueryRow(`SELECT incomplete FROM lmm_acquisition.activity_state WHERE singleton`).Scan(&incomplete); err != nil || !incomplete {
		t.Fatalf("legacy gap lost: %v %v", incomplete, err)
	}

	if _, err = db.Exec(`INSERT INTO lmm_acquisition.consents(user_id,allowed,version,updated_at) VALUES(77,false,1,$1) ON CONFLICT(user_id) DO UPDATE SET allowed=false`, s.now().Unix()); err != nil {
		t.Fatal(err)
	}
	if status := save(fmt.Sprintf(`{"expected_revision":%d,"source":"denied","reason":"operator correction"}`, revision)); status != 400 {
		t.Fatalf("withdrawn consent: %d", status)
	}

	if _, err = db.Exec(`UPDATE lmm_acquisition.consents SET allowed=true,version=2 WHERE user_id=77`); err != nil {
		t.Fatal(err)
	}
	s.UserRead = revokeActivityRead{testUserRead{"user"}, s}
	rebuild()
	if err = db.QueryRow(`SELECT (SELECT count(*) FROM lmm_acquisition.activities WHERE user_id=77)+(SELECT count(*) FROM lmm_acquisition.correction_heads WHERE user_id=77)`).Scan(&nativeRows); err != nil || nativeRows != 0 {
		t.Fatalf("withdrawn activity resurrected %d %v", nativeRows, err)
	}

	for _, tc := range []struct {
		role   string
		status int
	}{{"admin", 403}, {"superadmin", 403}, {"", 503}} {
		s.UserRead = testUserRead{tc.role}
		w := httptest.NewRecorder()
		rootMux.ServeHTTP(w, httptest.NewRequest("GET", "/api/admin/acquisition/users/77/corrections", nil))
		if w.Code != tc.status {
			t.Fatalf("role %q: %d", tc.role, w.Code)
		}
	}
	adminDenied := httptest.NewRecorder()
	mux.ServeHTTP(adminDenied, httptest.NewRequest("GET", "/api/admin/acquisition/users/77/corrections", nil))
	if adminDenied.Code != 403 {
		t.Fatalf("admin read user details: %d", adminDenied.Code)
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
