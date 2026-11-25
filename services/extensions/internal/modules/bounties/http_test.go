package bounties

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

type webAuthority struct {
	*recoveryAuthority
	approved      map[string]bool
	lostAck       bool
	denyDeveloper bool
}

func (a *webAuthority) Authorize(ctx context.Context, token string) (Actor, error) {
	if a.denyDeveloper {
		return Actor{}, ErrForbidden
	}
	return a.authority.Authorize(ctx, token)
}
func (a *webAuthority) SessionAuthorize(ctx context.Context, token string) (Actor, error) {
	return a.authority.Authorize(ctx, token)
}
func (a *webAuthority) PublicConfig(context.Context) (FeeConfig, error) {
	return FeeConfig{Enabled: true, Code: "configured", RateBasisPoints: 100, RatePercent: json.Number("1.00"), Limits: FeeLimits{1, 100, 1}}, nil
}
func (a *webAuthority) Preview(ctx context.Context, session string, c Command) (Receipt, error) {
	a.previews++
	if a.commands == nil {
		a.commands = map[string]Command{}
		a.receipts = map[string]Receipt{}
	}
	actor, e := a.authority.Authorize(ctx, session)
	if e != nil {
		return Receipt{}, e
	}
	c.Confirmation = ""
	a.commands[c.OperationID] = c
	r := Receipt{OperationID: c.OperationID, ProjectID: c.ProjectID, ChallengeID: c.ChallengeID, DisputeID: c.DisputeID, ActorUserID: actor.UserID, State: "awaiting_confirmation", Confirmation: "browser-" + c.OperationID, ConfirmationURL: "/core/v1/bounties/confirm/browser-" + c.OperationID, Expires: 9999999999}
	a.receipts[c.OperationID] = r
	return r, nil
}
func (a *webAuthority) Execute(ctx context.Context, session string, c Command) (Receipt, error) {
	if !a.approved[c.OperationID] {
		return Receipt{}, ErrForbidden
	}
	r, e := a.recoveryAuthority.Execute(ctx, session, c)
	if e != nil {
		return r, e
	}
	r.JournalIDs = []int64{int64(a.calls)}
	if c.Action == "publish" {
		r.Credit = c.RewardCredit * int64(c.RewardSlots)
		r.RewardCredit = c.RewardCredit - 1
		r.EscrowCredit = r.RewardCredit * int64(c.RewardSlots)
	}
	if c.Action == "tip" {
		r.Credit = c.Credit
	}
	a.receipts[c.OperationID] = r
	if a.lostAck {
		return Receipt{}, ErrUnknown
	}
	return r, nil
}
func webSetup(t *testing.T) (*Service, *webAuthority, http.Handler) {
	t.Helper()
	s, base := setup(t)
	a := &webAuthority{recoveryAuthority: &recoveryAuthority{authority: base}, approved: map[string]bool{}}
	s.Funds = a
	h, e := NewHTTP(s, SessionExtractorFunc(func(r *http.Request) (string, error) {
		v := r.Header.Get("X-Test-Session")
		if v == "" {
			return "", ErrForbidden
		}
		return v, nil
	}))
	if e != nil {
		t.Fatal(e)
	}
	return s, a, h
}
func webRequest(h http.Handler, method, path, session, body, key string) *httptest.ResponseRecorder {
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	if session != "" {
		r.Header.Set("X-Test-Session", session)
	}
	if key != "" {
		r.Header.Set("Idempotency-Key", key)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	return w
}
func dataMap(t *testing.T, w *httptest.ResponseRecorder) map[string]any {
	t.Helper()
	var out struct {
		Success bool           `json:"success"`
		Data    map[string]any `json:"data"`
	}
	if json.Unmarshal(w.Body.Bytes(), &out) != nil || !out.Success {
		t.Fatal(w.Code, w.Body.String())
	}
	return out.Data
}
func TestRetiredBountiesRejectNewBusiness(t *testing.T) {
	s, a, h := webSetup(t)
	for _, prefix := range []string{"/api/bounties", "/api/open-source-bounties"} {
		if w := webRequest(h, "GET", prefix+"/projects/p", "", "", ""); w.Code != 410 {
			t.Fatal("anonymous detail", w.Code)
		}
		for _, route := range [][2]string{
			{"GET", ""}, {"GET", "/config"}, {"POST", ""},
			{"PUT", "/projects/p"}, {"DELETE", "/projects/p"}, {"POST", "/projects/p/publish"},
			{"POST", "/projects/p/accept"}, {"POST", "/projects/p/resume"}, {"POST", "/projects/p/pause"},
			{"POST", "/projects/p/archive"}, {"POST", "/projects/p/unarchive"},
			{"POST", "/challenges/c/tip"}, {"POST", "/challenges/c/rate-owner"}, {"POST", "/tips/n/thank"},
		} {
			w := webRequest(h, route[0], prefix+route[1], "owner-token", "{}", "test")
			if w.Code != http.StatusGone {
				t.Fatalf("%v: %d %s", route, w.Code, w.Body.String())
			}
		}
	}
	for _, action := range Actions {
		switch action {
		case "list_accepted", "list_owned", "list_disputes":
			continue
		}
		if protectedAction(action) {
			continue
		}
		_, err := s.BuiltinInvoke(context.Background(), Invocation{Actor: Actor{Token: "owner-token"}, Command: Command{Action: action}})
		if err != ErrRetired {
			t.Fatalf("%s: %v", action, err)
		}
	}
	if a.previews != 0 || a.calls != 0 || len(s.Store.(*memory).records) != 0 {
		t.Fatal("retired action changed state")
	}
}

func seedPublished(t *testing.T, s *Service) {
	t.Helper()
	if _, err := invoke(t, s, "owner-token", "create_draft", "draft"); err != nil {
		t.Fatal(err)
	}
	err := s.Store.Within(context.Background(), func(tx Tx) error {
		var p Project
		if e := tx.Get("projects", "p", &p); e != nil {
			return e
		}
		p.Status = "published"
		return tx.Put("projects", "p", p)
	})
	if err != nil {
		t.Fatal(err)
	}
}
func TestHTTPFinancialPreviewRequiresBrowserAndUnknownUsesQueryRecovery(t *testing.T) {
	s, a, h := webSetup(t)
	seedPublished(t, s)
	w := webRequest(h, "POST", "/api/open-source-bounties/projects/p/close", "owner-token", "", "publish")
	if w.Code != 202 || a.calls != 0 || a.previews != 1 {
		t.Fatal(w.Code, w.Body.String(), a.calls, a.previews)
	}
	pending := dataMap(t, w)
	id := pending["operation_id"].(string)
	if pending["state"] != "awaiting_confirmation" || pending["confirmation_url"] == "" {
		t.Fatal(pending)
	}
	w = webRequest(h, "POST", "/api/open-source-bounties/operations/"+id+"/apply", "owner-token", "", "")
	if w.Code != 202 || a.calls != 0 {
		t.Fatal("unconfirmed request executed", w.Code, a.calls)
	}
	a.approved[id] = true
	a.lostAck = true
	w = webRequest(h, "POST", "/api/open-source-bounties/operations/"+id+"/apply", "owner-token", "", "")
	if w.Code != 503 || a.calls != 1 {
		t.Fatal(w.Code, w.Body.String(), a.calls)
	}
	a.lostAck = false
	a.denyDeveloper = true
	w = webRequest(h, "POST", "/api/open-source-bounties/operations/"+id+"/recover", "owner-token", "", "")
	if w.Code != 200 || a.calls != 1 || a.previews != 1 {
		t.Fatal("recovery repeated action or denied oldhold", w.Code, w.Body.String(), a.calls)
	}
	response := dataMap(t, w)
	project := response["project"].(map[string]any)
	if project["status"] != "closed" || project["reward_quota"] != float64(100) {
		t.Fatal(project)
	}
	w = webRequest(h, "POST", "/api/open-source-bounties/operations/"+id+"/apply", "worker-token", "", "")
	if w.Code != 403 {
		t.Fatal("foreign operation exposed")
	}
	s.Store.Within(context.Background(), func(tx Tx) error {
		var op WebOperation
		tx.Get("web_operations", id, &op)
		raw, _ := json.Marshal(op)
		if strings.Contains(string(raw), "owner-token") {
			t.Fatal("session persisted")
		}
		return nil
	})
}

func TestHTTPPreservesClientOperationIDAndResultReadIsReadOnly(t *testing.T) {
	s, a, h := webSetup(t)
	seedPublished(t, s)
	r := httptest.NewRequest("POST", "/api/open-source-bounties/projects/p/close", nil)
	r.Header.Set("X-Test-Session", "owner-token")
	r.Header.Set("X-Lmm-Operation-ID", "original-browser-operation")
	r.Header.Set("Idempotency-Key", "original-browser-operation")
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	if w.Code != 202 || dataMap(t, w)["operation_id"] != "original-browser-operation" {
		t.Fatal(w.Code, w.Body.String())
	}
	a.approved["original-browser-operation"] = true
	w = webRequest(h, "POST", "/api/open-source-bounties/operations/original-browser-operation/apply", "owner-token", "", "")
	if w.Code != 200 || a.calls != 1 {
		t.Fatal(w.Code, w.Body.String())
	}
	w = webRequest(h, "GET", "/api/open-source-bounties/operations/original-browser-operation/result", "owner-token", "", "")
	if w.Code != 200 || a.calls != 1 || dataMap(t, w)["project"] == nil {
		t.Fatal("result read executed operation", w.Code, w.Body.String())
	}
}
func TestRetiredConfirmedPublicationCannotDispatch(t *testing.T) {
	s, a, _ := webSetup(t)
	ctx := context.Background()
	invoke(t, s, "owner-token", "create_draft", "draft")
	cmd := Command{Action: "publish", ProjectID: "p", OperationID: "prepared-only", RewardCredit: 100, RewardSlots: 2, RepositoryURL: "https://github.com/owner/repo"}
	a.Preview(ctx, "owner-token", cmd)
	in := Invocation{Command: Command{Action: "publish", ProjectID: "p", OperationID: "prepared-only"}}
	s.Store.Within(ctx, func(tx Tx) error {
		return tx.Put("web_operations", "prepared-only", WebOperation{ID: "prepared-only", ActorUserID: "owner", Invocation: in, State: "preparing"})
	})
	for _, approved := range []bool{false, true} {
		a.approved["prepared-only"] = approved
		for _, recoverOnly := range []bool{true, false} {
			op, e := s.ApplyWeb(ctx, "owner-token", "prepared-only", recoverOnly)
			if e != ErrRetired || op.State != "preparing" || a.calls != 0 {
				t.Fatal(op, e, a.calls)
			}
		}
	}
}

func (a *webAuthority) QueryForCommand(ctx context.Context, session string, c Command) (Receipt, error) {
	r, e := a.recoveryAuthority.QueryForCommand(ctx, session, c)
	if e == nil && pendingReceipt(r) && a.approved[c.OperationID] {
		r.State = "confirmed"
		r.Status = "confirmed"
	}
	return r, e
}
