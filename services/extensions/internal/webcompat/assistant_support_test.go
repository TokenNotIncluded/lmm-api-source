// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
)

type supportPort struct {
	token        string
	conversation int64
	input        assistant_services.WebSupportInput
	turn         string
	cancel       bool
	err          error
}

func (p *supportPort) WebSupportEligibility(_ context.Context, token string) (bool, error) {
	p.token = token
	return true, p.err
}
func (p *supportPort) WebSupportSelf(_ context.Context, token string, conversation int64) (*assistant_services.WebSupportRequest, error) {
	p.token, p.conversation = token, conversation
	if conversation == 0 {
		return nil, p.err
	}
	return &assistant_services.WebSupportRequest{ID: 3, ConversationID: conversation, Kind: "handoff", Status: "pending"}, p.err
}
func (p *supportPort) WebCreateSupport(_ context.Context, token string, in assistant_services.WebSupportInput) (assistant_services.WebSupportRequest, bool, error) {
	p.token, p.input = token, in
	return assistant_services.WebSupportRequest{ID: 4, Kind: in.Kind, Status: "pending", ConversationID: in.ConversationID}, true, p.err
}
func (p *supportPort) WebSupportDetail(_ context.Context, token string, id int64) (assistant_services.WebSupportRequest, []assistant_services.WebSupportMessage, error) {
	p.token = token
	return assistant_services.WebSupportRequest{ID: id, Status: "accepted"}, []assistant_services.WebSupportMessage{{ID: 1, Role: "user", Content: "hi", CreatedAt: 9}}, p.err
}
func (p *supportPort) WebSendSupportMessage(_ context.Context, token string, id int64, turn, content string) (assistant_services.WebSupportMessage, error) {
	p.token, p.turn = token, turn
	return assistant_services.WebSupportMessage{ID: 2, Role: "user", Content: content}, p.err
}
func (p *supportPort) WebAcceptSupport(_ context.Context, token string, id int64) (assistant_services.WebSupportRequest, error) {
	p.token = token
	return assistant_services.WebSupportRequest{ID: id, Status: "accepted"}, p.err
}
func (p *supportPort) WebCloseSupport(_ context.Context, token string, id int64, cancel bool) (assistant_services.WebSupportRequest, error) {
	p.token, p.cancel = token, cancel
	return assistant_services.WebSupportRequest{ID: id, Status: "cancelled"}, p.err
}

func decodeSupport(t *testing.T, body []byte) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal(body, &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func TestAssistantSupportOriginalRoutesUseCurrentSessionAndEnvelope(t *testing.T) {
	port := &supportPort{}
	mux := http.NewServeMux()
	RegisterAssistantSupport(mux, newAuth(t, &authCore{}), port)
	for _, tc := range []struct{ method, path, body, key string }{
		{"GET", "/api/assistant/support/eligibility", "", "eligible"},
		{"GET", "/api/assistant/support/self?conversation_id=7", "", "request"},
		{"POST", "/api/assistant/support", `{"conversation_id":7,"kind":"handoff","topic":"help"}`, "created"},
		{"GET", "/api/assistant/support/5", "", "messages"},
		{"POST", "/api/assistant/support/5/messages", `{"content":"hello","client_turn_id":"turn_1234"}`, "message"},
		{"POST", "/api/assistant/support/5/accept", `{}`, "request"},
		{"POST", "/api/assistant/support/5/close", `{"cancel":true}`, "request"},
	} {
		port.token = ""
		w := assistantRequest(mux, tc.method, tc.path, webToken, tc.body)
		out := decodeSupport(t, w.Body.Bytes())
		data, _ := out["data"].(map[string]any)
		if w.Code != 200 || out["success"] != true || data == nil || data[tc.key] == nil {
			t.Fatalf("%s %s: %d %s", tc.method, tc.path, w.Code, w.Body)
		}
		if port.token != webToken {
			t.Fatalf("%s %s did not pass the current session", tc.method, tc.path)
		}
	}
	if port.conversation != 7 || port.input.ConversationID != 7 || port.input.Kind != "handoff" || port.turn != "turn_1234" || !port.cancel {
		t.Fatalf("inputs not forwarded: %+v", port)
	}
	// No active request is an explicit null, as the original route returned.
	w := assistantRequest(mux, "GET", "/api/assistant/support/self", webToken, "")
	if out := decodeSupport(t, w.Body.Bytes()); w.Code != 200 || out["data"].(map[string]any)["request"] != nil {
		t.Fatalf("self without request: %d %s", w.Code, w.Body)
	}
}

func TestAssistantSupportRejectsMissingSessionAndMapsErrors(t *testing.T) {
	port := &supportPort{}
	mux := http.NewServeMux()
	RegisterAssistantSupport(mux, newAuth(t, &authCore{}), port)
	if w := assistantRequest(mux, "GET", "/api/assistant/support/eligibility", "", ""); w.Code != http.StatusUnauthorized || port.token != "" {
		t.Fatalf("missing session reached service: %d", w.Code)
	}
	for _, tc := range []struct {
		err   error
		code  int
		label string
	}{
		{assistant_services.ErrSupportIneligible, 403, "ASSISTANT_SUPPORT_RECHARGE_REQUIRED"},
		{access.ErrForbidden, 403, "ASSISTANT_SUPPORT_FORBIDDEN"},
		{access.ErrNotFound, 404, "ASSISTANT_SUPPORT_NOT_FOUND"},
		{access.ErrConflict, 409, "ASSISTANT_SUPPORT_CONFLICT"},
		{access.ErrInvalid, 422, "ASSISTANT_SUPPORT_INVALID"},
		{access.ErrUnavailable, 503, "ASSISTANT_SUPPORT_UNAVAILABLE"},
	} {
		port.err = tc.err
		w := assistantRequest(mux, "POST", "/api/assistant/support", webToken, `{"kind":"appointment"}`)
		if out := decodeSupport(t, w.Body.Bytes()); w.Code != tc.code || out["code"] != tc.label || out["success"] != false {
			t.Fatalf("%v: %d %s", tc.err, w.Code, w.Body)
		}
	}
	port.err = nil
	for _, path := range []string{"/api/assistant/support/self?conversation_id=-1", "/api/assistant/support/self?conversation_id=x"} {
		if w := assistantRequest(mux, "GET", path, webToken, ""); w.Code != 422 {
			t.Fatalf("%s: %d", path, w.Code)
		}
	}
	if w := assistantRequest(mux, "GET", "/api/assistant/support/0", webToken, ""); w.Code != 404 {
		t.Fatalf("zero id: %d", w.Code)
	}
	if w := assistantRequest(mux, "POST", "/api/assistant/support", webToken, `{"kind":"handoff","user_id":9}`); w.Code != 422 {
		t.Fatalf("unknown field accepted: %d", w.Code)
	}
}

// Mount discovers the routes by asserting this port on the services handler.
var _ AssistantSupport = (*assistant_services.Service)(nil)
var _ http.Handler = (*assistant_services.Service)(nil)
