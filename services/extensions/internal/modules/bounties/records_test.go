package bounties

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestRecordsArchiveContentAndAnonymousPrivacy(t *testing.T) {
	s, _ := setup(t)
	ctx := context.Background()
	invoke(t, s, "owner-token", "create_draft", "create")
	if _, e := s.Detail(ctx, "", "p"); e != ErrRetired {
		t.Fatal(e)
	}
	for _, action := range []string{"content", "archive", "unarchive"} {
		if _, e := s.Records(ctx, "owner-token", RecordRequest{Action: action, ProjectID: "p", Draft: draft()}); e != ErrRetired {
			t.Fatal(action, e)
		}
	}
	if _, e := s.Detail(ctx, "worker-token", "p"); e != ErrNotFound {
		t.Fatal("foreign detail", e)
	}
	if _, e := s.Detail(ctx, "owner-token", "p"); e != nil {
		t.Fatal("historical owner detail", e)
	}

}
func TestReceiptNotificationsOwnershipReadThankAndLedger(t *testing.T) {
	s, _ := setup(t)
	ctx := context.Background()
	p := Project{ID: "p", OwnerUserID: "owner", Draft: Draft{Title: "Native bounty"}}
	ch := Challenge{ID: "ch", ProjectID: "p", ParticipantUserID: "worker"}
	cmd := Command{Action: "tip", OperationID: "tip-op", ProjectID: "p", ChallengeID: "ch", ReviewNote: "thanks"}
	receipt := Receipt{OperationID: "tip-op", Credit: 7, JournalIDs: []int64{42}}
	if e := s.Store.Within(ctx, func(tx Tx) error {
		tx.Put("projects", "p", p)
		tx.Put("challenges", "ch", ch)
		return s.projectNotification(tx, Actor{UserID: "owner"}, cmd, receipt, p, ch, 100)
	}); e != nil {
		t.Fatal(e)
	}
	s.Store.Within(ctx, func(tx Tx) error { return s.projectNotification(tx, Actor{UserID: "owner"}, cmd, receipt, p, ch, 101) })
	out, e := s.Records(ctx, "worker-token", RecordRequest{Action: "tips_received"})
	if e != nil || len(out.Items) != 1 {
		t.Fatal(out, e)
	}
	for _, token := range []string{"owner-token", "worker-token"} {
		if _, e = s.Records(ctx, token, RecordRequest{Action: "thank_tip", NotificationID: "tip-op"}); e != ErrRetired {
			t.Fatal(e)
		}
	}
	var ledger []json.RawMessage
	s.Store.Within(ctx, func(tx Tx) error { ledger, _ = tx.(ViewTx).All("ledger", Query{ProjectID: "p"}); return nil })
	if len(ledger) != 1 || !strings.Contains(string(ledger[0]), "42") {
		t.Fatal("receipt ledger duplicated/lost", ledger)
	}
	s.Store.Within(ctx, func(tx Tx) error {
		return tx.Put("notifications", "reward", Notification{ID: "reward", RecipientUserID: "worker", Kind: "reward_transfer", Quota: 99})
	})
	if _, e = s.Records(ctx, "worker-token", RecordRequest{Action: "tips_read"}); e != nil {
		t.Fatal(e)
	}
	s.Store.Within(ctx, func(tx Tx) error {
		var n Notification
		tx.Get("notifications", "reward", &n)
		if n.RecipientReadAt != 0 {
			t.Fatal("tips read marked other reward")
		}
		return nil
	})
	if _, e = s.Records(ctx, "worker-token", RecordRequest{Action: "notifications_read"}); e != nil {
		t.Fatal(e)
	}
}
