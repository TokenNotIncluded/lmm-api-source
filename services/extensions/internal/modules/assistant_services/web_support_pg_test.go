package assistant_services

import (
	"context"
	"errors"
	"fmt"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type pgWebEligibility struct{ eligible bool }

func (p *pgWebEligibility) SupportEligibility(context.Context, string) (Eligibility, error) {
	return Eligibility{Eligible: p.eligible, ReceiptID: "synthetic-eligibility", CheckedAt: time.Now().Unix()}, nil
}

// The original /api/assistant/support DTOs over the real restricted runtime.
func TestWebSupportOriginalContractPG(t *testing.T) {
	ctx := context.Background()
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	eligibility := &pgWebEligibility{}
	s, err := New(ctx, runtime, pgServiceAuthority{}, Dependencies{SupportEligibility: eligibility})
	if err != nil {
		t.Fatal(err)
	}
	const owner, other = "session-user-202", "session-user-101"
	var conversation int64
	if err = runtime.QueryRowContext(ctx, `INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES(202,'Support','Support',1,1) RETURNING id`).Scan(&conversation); err != nil {
		t.Fatal(err)
	}
	if got, err := s.WebSupportSelf(ctx, owner, conversation); err != nil || got != nil {
		t.Fatal("unexpected request before creation", got, err)
	}
	if _, err := s.WebSupportSelf(ctx, other, conversation); !errors.Is(err, access.ErrNotFound) {
		t.Fatal("foreign conversation was readable", err)
	}
	if eligible, err := s.WebSupportEligibility(ctx, owner); err != nil || eligible {
		t.Fatal("eligibility not forwarded", eligible, err)
	}
	if _, _, err := s.WebCreateSupport(ctx, owner, WebSupportInput{ConversationID: conversation, Kind: "appointment", Topic: "Install", PreferredTime: "Tomorrow UTC", ScheduledAt: time.Now().Unix() + 3600}); !errors.Is(err, ErrSupportIneligible) {
		t.Fatal("ineligible appointment accepted", err)
	}
	created, ok, err := s.WebCreateSupport(ctx, owner, WebSupportInput{ConversationID: conversation, Kind: "handoff", Topic: "Need a human"})
	if err != nil || !ok || created.ConversationID != conversation || created.UserID != 202 || created.Status != "pending" || created.AssignedAdminID != 0 || created.ClosedAt != 0 {
		t.Fatalf("handoff not created with Web DTO: %+v %v", created, err)
	}
	again, ok, err := s.WebCreateSupport(ctx, owner, WebSupportInput{Kind: "handoff"})
	if err != nil || ok || again.ID != created.ID {
		t.Fatal("active request was duplicated", err)
	}
	for _, conv := range []int64{0, conversation} {
		self, err := s.WebSupportSelf(ctx, owner, conv)
		if err != nil || self == nil || self.ID != created.ID || self.ConversationID != conversation {
			t.Fatal("self did not return the active request", conv, err)
		}
	}
	if _, _, err := s.WebSupportDetail(ctx, other, created.ID); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("another user read the request", err)
	}
	queue, messages, err := s.WebSupportDetail(ctx, "session-admin-502", created.ID)
	if err != nil || queue.ID != created.ID || len(messages) != 0 {
		t.Fatal("unassigned administrator queue view differs", err, len(messages))
	}
	accepted, err := s.WebAcceptSupport(ctx, "session-admin-501", created.ID)
	if err != nil || accepted.Status != "accepted" || accepted.AssignedAdminID != 501 || accepted.AcceptedAt == 0 {
		t.Fatal("accept failed", err)
	}
	if _, _, err := s.WebSupportDetail(ctx, "session-admin-502", created.ID); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("other administrator read an assigned request", err)
	}
	if m, err := s.WebSendSupportMessage(ctx, owner, created.ID, "", "Still broken"); err != nil || m.Role != "user" {
		t.Fatal("message without client turn failed", err)
	}
	if _, err := s.WebSendSupportMessage(ctx, "session-admin-501", created.ID, "human-web-0001", "Looking now"); err != nil {
		t.Fatal(err)
	}
	_, messages, err = s.WebSupportDetail(ctx, owner, created.ID)
	if err != nil || len(messages) != 2 || messages[0].Role != "user" || messages[1].Role != "human" {
		t.Fatal("owner transcript differs", err, messages)
	}
	if _, err := s.WebCloseSupport(ctx, owner, created.ID, false); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("owner completed instead of cancelling", err)
	}
	closed, err := s.WebCloseSupport(ctx, owner, created.ID, true)
	if err != nil || closed.Status != "cancelled" || closed.ClosedAt == 0 || closed.ConversationID != conversation {
		t.Fatal("cancel failed", err)
	}
	if self, err := s.WebSupportSelf(ctx, owner, 0); err != nil || self != nil {
		t.Fatal("closed request still active", err)
	}
	if self, err := s.WebSupportSelf(ctx, owner, conversation); err != nil || self == nil || self.Status != "cancelled" {
		t.Fatal("latest conversation request missing", err)
	}
	eligibility.eligible = true
	standalone, ok, err := s.WebCreateSupport(ctx, owner, WebSupportInput{Kind: "appointment", Topic: "Install", PreferredTime: "Tomorrow UTC", ScheduledAt: time.Now().Unix() + 3600})
	if err != nil || !ok || standalone.ConversationID != 0 || standalone.Kind != "appointment" {
		t.Fatal("standalone eligible appointment failed", err)
	}
	if _, err := s.WebCloseSupport(ctx, owner, standalone.ID, true); err != nil {
		t.Fatal(err)
	}
	// A registered Core conversation is stored under its Core ID, so the agent
	// pause boundary applies, and still reports the numeric Web conversation.
	var registered int64
	if err = runtime.QueryRowContext(ctx, `INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES(202,'Agent','Agent',1,1) RETURNING id`).Scan(&registered); err != nil {
		t.Fatal(err)
	}
	core := fmt.Sprintf("lmm_cv_%064x", time.Now().UnixNano())
	if _, err = runtime.ExecContext(ctx, `INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,core_conversation_id,phase) VALUES(202,$1,$2,$3,'registered')`, registered, fmt.Sprintf("creation-%d", registered), core); err != nil {
		t.Fatal(err)
	}
	mapped, _, err := s.WebCreateSupport(ctx, owner, WebSupportInput{ConversationID: registered, Kind: "handoff"})
	if err != nil || mapped.ConversationID != registered {
		t.Fatal("registered conversation mapping differs", err)
	}
	if paused, err := s.HumanSupportActive(ctx, owner, core); err != nil || !paused {
		t.Fatal("Web handoff did not pause the Core conversation", err)
	}
	if _, err := s.WebCloseSupport(ctx, owner, mapped.ID, true); err != nil {
		t.Fatal(err)
	}
}
