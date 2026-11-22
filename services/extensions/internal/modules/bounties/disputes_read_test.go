package bounties

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
)

type disputeReadRepo struct {
	*observedReadRepository
	after func()
}

func (r *disputeReadRepo) Within(ctx context.Context, fn func(Tx) error) error {
	err := r.observedReadRepository.Within(ctx, fn)
	if r.after != nil {
		r.after()
	}
	return err
}

func disputeReadFixture(t *testing.T) (*Service, *disputeReadRepo, *atomic.Bool, *atomic.Bool, *atomic.Int64, *atomic.Int32) {
	t.Helper()
	admin, denied, user := &atomic.Bool{}, &atomic.Bool{}, &atomic.Int64{}
	user.Store(7)
	client, calls := clientFixture(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/bounties/v1/authorize" {
			t.Error("read reached a money/confirmation or non-policy endpoint", r.URL.Path)
			w.WriteHeader(500)
			return
		}
		if denied.Load() {
			w.WriteHeader(403)
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"user_id": user.Load(), "admin": admin.Load()})
	})
	base := &memory{records: map[string]json.RawMessage{}}
	err := base.Within(context.Background(), func(tx Tx) error {
		for _, spec := range []struct{ id, owner, participant, status string }{
			{"own-owner", "7", "8", "open"}, {"own-participant", "8", "7", "resolved_paid"}, {"unrelated", "8", "9", "open"},
		} {
			p := Project{ID: "p-" + spec.id, OwnerUserID: spec.owner, Status: "published", EscrowCredit: 898, NetRewardCredit: 101, CreatedAt: 90,
				Draft: Draft{Title: "Current title", RepositoryURL: "https://github.com/example/project", Rules: "Current acceptance rules", RewardCredit: 101, RewardSlots: 1}}
			ch := Challenge{ID: "c-" + spec.id, ProjectID: p.ID, OwnerUserID: spec.owner, ParticipantUserID: spec.participant, Status: "rejected", RewardCredit: 101, CreatedAt: 95,
				IssueURL: "https://github.com/example/project/issues/1", SubmissionNote: "Real submitted evidence", Command: Command{Confirmation: "nested-private-confirmation"}}
			d := Dispute{ID: "d-" + spec.id, ProjectID: p.ID, ChallengeID: ch.ID, OwnerUserID: spec.owner, ParticipantUserID: spec.participant,
				OpenedByUserID: spec.participant, Status: spec.status, ProjectSnapshot: p, ChallengeSnapshot: ch, TipCreditSnapshot: 3,
				Reason: "Disagreement", Statement: "Recorded statement", CreatedAt: 100, UpdatedAt: 102, Command: Command{Action: "open_dispute", Confirmation: "private-confirmation"}}
			d.ProjectSnapshot.Title, d.ProjectSnapshot.EscrowCredit = "Original title", 999
			if spec.status != "open" {
				d.ResolvedAt, d.ResolvedByUserID, d.Resolution = 101, "11", "Reviewed resolution"
			}
			for _, rec := range []struct {
				bucket, id string
				value      any
			}{
				{"projects", p.ID, p}, {"challenges", ch.ID, ch}, {"disputes", d.ID, d},
				{"notifications", "n-" + spec.id, Notification{ID: "n-" + spec.id, ProjectID: p.ID, ChallengeID: ch.ID, Kind: "tip_transfer", RecipientUserID: spec.participant, SenderUserID: spec.owner, Quota: 7}},
			} {
				if err := tx.Put(rec.bucket, rec.id, rec.value); err != nil {
					return err
				}
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	repo := &disputeReadRepo{observedReadRepository: &observedReadRepository{base: base}}
	return &Service{Store: repo, Funds: client}, repo, admin, denied, user, calls
}

func TestDisputesForViewerRealDomainPartyAndAdministratorScopes(t *testing.T) {
	s, repo, admin, _, _, calls := disputeReadFixture(t)
	before, _ := json.Marshal(repo.base.records)
	ctx := context.Background()
	for _, currentAdmin := range []bool{false, true} {
		admin.Store(currentAdmin)
		rows, err := s.DisputesForViewer(ctx, "session-private", false, "", 50)
		if err != nil || len(rows) != 2 {
			t.Fatal("personal view changed with admin status", err, len(rows))
		}
		q := repo.queries[len(repo.queries)-1]
		if q.PartyUserID != "7" || q.OwnerUserID != "" || q.ParticipantUserID != "" || q.Limit != 50 {
			t.Fatal("personal query not party-scoped", q)
		}
		for _, row := range rows {
			if row["owner_user_id"] != "7" && row["participant_user_id"] != "7" {
				t.Fatal("unrelated dispute disclosed", row["id"])
			}
			if row["project_escrow_quota_snapshot"] != int64(999) || row["current_project_escrow_quota"] != int64(898) || row["tip_quota_snapshot"] != int64(3) || row["tip_quota"] != int64(7) || row["reward_quota"] != int64(101) || row["live_evidence_changed"] != true {
				t.Fatal("flat money/source facts changed", row["id"])
			}
			if deadline, present := row["resolution_deadline_at"]; !present || deadline != nil {
				t.Fatal("invented resolution deadline")
			}
			for _, key := range []string{"evidence", "project_snapshot", "challenge_snapshot"} {
				if _, found := row[key]; found {
					t.Fatal("private command record escaped", key)
				}
			}
			serialized, _ := json.Marshal(row)
			if strings.Contains(string(serialized), "private-confirmation") {
				t.Fatal("confirmation escaped")
			}
		}
	}
	admin.Store(true)
	rows, err := s.DisputesForViewer(ctx, "session-private", true, "open", 100)
	if err != nil || len(rows) != 2 {
		t.Fatal("actual admin/filter failed", err, len(rows))
	}
	q := repo.queries[len(repo.queries)-1]
	if q.PartyUserID != "" || q.Status != "open" || q.Limit != 100 {
		t.Fatal(q)
	}
	seenUnrelated := false
	for _, row := range rows {
		if row["id"] == "d-unrelated" {
			seenUnrelated = true
		}
	}
	if !seenUnrelated {
		t.Fatal("admin could not read actual unrelated dispute")
	}
	rows, err = s.DisputesForViewer(ctx, "session-private", false, "resolved_paid", 1)
	if err != nil || len(rows) != 1 || rows[0]["id"] != "d-own-participant" {
		t.Fatal("personal status/limit changed", err)
	}
	after, _ := json.Marshal(repo.base.records)
	if repo.writes != 0 || !reflect.DeepEqual(before, after) || calls.Load() != 8 {
		t.Fatal("read wrote or bypassed fresh authority", repo.writes, calls.Load())
	}
}

func TestDisputesForViewerRejectsMissingPolicyBeforeRepositoryAndDowngradeBeforeReturn(t *testing.T) {
	for _, mode := range []string{"ordinary-admin", "policy-denied", "mid-read-demotion", "mid-read-revocation", "mid-read-other-user"} {
		t.Run(mode, func(t *testing.T) {
			s, repo, admin, denied, user, calls := disputeReadFixture(t)
			admin.Store(true)
			expectedQueries := 1
			switch mode {
			case "ordinary-admin":
				admin.Store(false)
				expectedQueries = 0
			case "policy-denied":
				denied.Store(true)
				expectedQueries = 0
			case "mid-read-demotion":
				repo.after = func() { admin.Store(false) }
			case "mid-read-revocation":
				repo.after = func() { denied.Store(true) }
			case "mid-read-other-user":
				repo.after = func() { user.Store(8) }
			}
			before, _ := json.Marshal(repo.base.records)
			rows, err := s.DisputesForViewer(context.Background(), "session-private", true, "", 50)
			after, _ := json.Marshal(repo.base.records)
			if !errors.Is(err, ErrForbidden) || rows != nil || len(repo.queries) != expectedQueries || repo.writes != 0 || !reflect.DeepEqual(before, after) || calls.Load() != int32(expectedQueries+1) {
				t.Fatal("authority failure released data or wrote", mode, err, len(rows), len(repo.queries), calls.Load())
			}
		})
	}
}

func TestDisputesForViewerRejectsInvalidFiltersWithoutAuthorityOrWrites(t *testing.T) {
	for _, spec := range []struct {
		status string
		limit  int
	}{{"paid", 50}, {" open ", 50}, {"open", 0}, {"", 101}} {
		s, repo, _, _, _, calls := disputeReadFixture(t)
		if _, err := s.DisputesForViewer(context.Background(), "session-private", false, spec.status, spec.limit); !errors.Is(err, ErrInvalid) || calls.Load() != 0 || len(repo.queries) != 0 || repo.writes != 0 {
			t.Fatal(spec, err)
		}
	}
}

func TestDisputesForViewerRawAmountsCannotDefaultToZero(t *testing.T) {
	for _, spec := range []struct{ key, nested, field string }{
		{"disputes:d-own-owner", "", "tip_quota_snapshot"},
		{"disputes:d-own-owner", "project_snapshot", "escrow_credit"},
		{"disputes:d-own-owner", "challenge_snapshot", "reward_credit"},
		{"projects:p-own-owner", "", "escrow_credit"},
		{"challenges:c-own-owner", "", "reward_credit"},
	} {
		for _, mode := range []string{"missing", "null", "negative"} {
			t.Run(spec.key+"/"+spec.nested+"/"+spec.field+"/"+mode, func(t *testing.T) {
				s, repo, _, _, _, _ := disputeReadFixture(t)
				var record map[string]any
				if json.Unmarshal(repo.base.records[spec.key], &record) != nil {
					t.Fatal("fixture")
				}
				fields := record
				if spec.nested != "" {
					fields = record[spec.nested].(map[string]any)
				}
				switch mode {
				case "missing":
					delete(fields, spec.field)
				case "null":
					fields[spec.field] = nil
				case "negative":
					fields[spec.field] = -1
				}
				repo.base.records[spec.key], _ = json.Marshal(record)
				before, _ := json.Marshal(repo.base.records)
				rows, err := s.DisputesForViewer(context.Background(), "session-private", false, "open", 50)
				after, _ := json.Marshal(repo.base.records)
				if !errors.Is(err, ErrUnavailable) || rows != nil || repo.writes != 0 || !reflect.DeepEqual(before, after) {
					t.Fatal("incomplete money became zero", err)
				}
			})
		}
	}
}

func TestDisputesForViewerRejectsCrossLinkedLiveRecords(t *testing.T) {
	s, repo, _, _, _, _ := disputeReadFixture(t)
	var ch Challenge
	json.Unmarshal(repo.base.records["challenges:c-own-owner"], &ch)
	ch.ParticipantUserID = "unrelated"
	repo.base.records["challenges:c-own-owner"], _ = json.Marshal(ch)
	rows, err := s.DisputesForViewer(context.Background(), "session-private", false, "open", 50)
	if !errors.Is(err, ErrConflict) || rows != nil || repo.writes != 0 {
		t.Fatal("cross-linked private evidence accepted", err)
	}
}
