package bounties

import (
	"context"
	"encoding/json"
	"net/http"
	"reflect"
	"sync/atomic"
	"testing"
)

// These cases exercise the existing dispatcher and private UDS client, not a
// substitute action implementation. The transport fixture returns one ordinary
// current Session. No developer entitlement is asserted or required.
type observedReadRepository struct {
	base    *memory
	writes  int
	queries []Query
}
type observedReadTx struct {
	*memoryTx
	parent *observedReadRepository
}

func (r *observedReadRepository) Within(ctx context.Context, fn func(Tx) error) error {
	return r.base.Within(ctx, func(tx Tx) error { return fn(&observedReadTx{tx.(*memoryTx), r}) })
}
func (tx *observedReadTx) Put(kind, id string, value any) error {
	tx.parent.writes++
	return tx.memoryTx.Put(kind, id, value)
}
func (tx *observedReadTx) List(kind string, q Query) ([]json.RawMessage, error) {
	tx.parent.queries = append(tx.parent.queries, q)
	return tx.memoryTx.List(kind, q)
}

func TestAssistantBountyLiteralPrivateReadsUseCurrentSessionOwnerAndNoWrites(t *testing.T) {
	var revoked atomic.Bool
	c, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/bounties/v1/session-authorize" {
			t.Error("read invoked money or a different authority", r.URL.Path)
			w.WriteHeader(500)
			return
		}
		if revoked.Load() {
			w.WriteHeader(403)
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"user_id": 7, "admin": false})
	})
	base := &memory{records: map[string]json.RawMessage{}}
	ctx := context.Background()
	if err := base.Within(ctx, func(tx Tx) error {
		for _, p := range []Project{{ID: "own-1", OwnerUserID: "7", Status: "draft"}, {ID: "own-2", OwnerUserID: "7", Status: "published"}, {ID: "other", OwnerUserID: "8", Status: "published"}} {
			if err := tx.Put("projects", p.ID, p); err != nil {
				return err
			}
		}
		for _, c := range []Challenge{{ID: "mine", ProjectID: "other", ParticipantUserID: "7", Status: "accepted"}, {ID: "other", ProjectID: "own-1", ParticipantUserID: "8", Status: "accepted"}} {
			if err := tx.Put("challenges", c.ID, c); err != nil {
				return err
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	before, err := json.Marshal(base.records)
	if err != nil {
		t.Fatal(err)
	}
	repo := &observedReadRepository{base: base}
	s := &Service{Store: repo, Funds: c}
	for _, action := range []string{"list_owned", "list_accepted"} {
		// A forged input identity/admin flag is replaced by Core's current actor.
		out, err := s.BuiltinInvoke(ctx, Invocation{Actor: Actor{Token: "session-private", UserID: "8", Admin: true}, Command: Command{Action: action}, Page: 1, PageSize: 10})
		if err != nil || out.Receipt != nil || len(out.Items) == 0 {
			t.Fatal(action, err, out)
		}
		for _, raw := range out.Items {
			if action == "list_owned" {
				var p Project
				if json.Unmarshal(raw, &p) != nil || p.OwnerUserID != "7" {
					t.Fatal("other owner's project returned", string(raw))
				}
			} else {
				var ch Challenge
				if json.Unmarshal(raw, &ch) != nil || ch.ParticipantUserID != "7" {
					t.Fatal("other user's challenge returned", string(raw))
				}
			}
		}
	}
	if calls.Load() != 2 || len(repo.queries) != 2 || repo.queries[0].OwnerUserID != "7" || repo.queries[0].PartyUserID != "" || repo.queries[1].ParticipantUserID != "7" {
		t.Fatal("scope not derived from current Session", repo.queries, calls.Load())
	}
	revoked.Store(true)
	if _, err = s.BuiltinInvoke(ctx, Invocation{Actor: Actor{Token: "session-private"}, Command: Command{Action: "list_owned"}}); err != ErrForbidden || len(repo.queries) != 2 {
		t.Fatal("revoked Session reached repository", err)
	}
	after, err := json.Marshal(base.records)
	if err != nil {
		t.Fatal(err)
	}
	if repo.writes != 0 || string(before) != string(after) {
		t.Fatal("private read mutated state", repo.writes)
	}
}

func TestAssistantBountyBoardAndDetailKeepActualDomainVisibility(t *testing.T) {
	s, a := setup(t)
	ctx := context.Background()
	published := Project{ID: "public", OwnerUserID: "owner", Status: "published", Draft: draft()}
	draftProject := published
	draftProject.ID = "private"
	draftProject.Status = "draft"
	archived := published
	archived.ID = "archived"
	archived.ArchivedAt = 1
	if err := s.Store.Within(ctx, func(tx Tx) error {
		for _, p := range []Project{published, draftProject, archived} {
			if err := tx.Put("projects", p.ID, p); err != nil {
				return err
			}
		}
		if err := tx.Put("challenges", "mine", Challenge{ID: "mine", ProjectID: "public", ParticipantUserID: "worker", OwnerUserID: "owner", Status: "accepted"}); err != nil {
			return err
		}
		if err := tx.Put("challenges", "other", Challenge{ID: "other", ProjectID: "public", ParticipantUserID: "other", OwnerUserID: "owner", Status: "accepted"}); err != nil {
			return err
		}
		return tx.Put("ledger", "posted", LedgerEntry{ID: "posted", ProjectID: "public", UserID: "owner", Quota: 100, JournalIDs: []int64{42}})
	}); err != nil {
		t.Fatal(err)
	}
	base := s.Store.(*memory)
	before, err := json.Marshal(base.records)
	if err != nil {
		t.Fatal(err)
	}
	board, err := s.BoardFor(ctx, "worker-token", 1, 20)
	if err != nil {
		t.Fatal(err)
	}
	items := board["items"].([]map[string]any)
	if len(items) != 1 || items[0]["id"] != "public" || items[0]["viewer_challenge"].(map[string]any)["participant_user_id"] != "worker" {
		t.Fatal(board)
	}
	for _, id := range []string{"private", "archived"} {
		if _, err = s.Detail(ctx, "worker-token", id); err != ErrNotFound {
			t.Fatal("private project disclosed", id, err)
		}
	}
	other, err := s.Detail(ctx, "worker-token", "public")
	if err != nil || len(other["challenges"].([]map[string]any)) != 0 || len(other["ledger"].([]json.RawMessage)) != 0 {
		t.Fatal(other, err)
	}
	owner, err := s.Detail(ctx, "owner-token", "public")
	if err != nil || len(owner["challenges"].([]map[string]any)) != 2 || len(owner["ledger"].([]json.RawMessage)) != 1 {
		t.Fatal(owner, err)
	}
	if _, err = s.Detail(ctx, "owner-token", "private"); err != nil {
		t.Fatal("owner lost private preview", err)
	}
	after, err := json.Marshal(base.records)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(before, after) || a.calls != 0 {
		t.Fatal("domain read changed state or executed funds")
	}
}
