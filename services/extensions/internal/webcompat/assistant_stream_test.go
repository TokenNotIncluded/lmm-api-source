// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"bufio"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
)

type assistantStreamHTTPFixture struct {
	assistantChatFixture
	release chan struct{}
	closed  chan struct{}
}

func (*assistantStreamHTTPFixture) StreamReady(context.Context) error { return nil }
func (p *assistantStreamHTTPFixture) StreamChat(ctx context.Context, token string, in assistant.PersonalChatRequest, consume func(assistant.AgentOutput) error) (assistant.PersonalChatReply, error) {
	defer close(p.closed)
	if err := consume(assistant.AgentOutput{ConversationID: 42, ClientTurnID: in.ClientTurnID, Sequence: 1, Phase: "model", Text: "first prefix"}); err != nil {
		return assistant.PersonalChatReply{}, err
	}
	select {
	case <-ctx.Done():
		return assistant.PersonalChatReply{}, ctx.Err()
	case <-p.release:
	}
	if err := consume(assistant.AgentOutput{ConversationID: 42, ClientTurnID: in.ClientTurnID, Sequence: 2, Phase: "model", Text: "first prefix plus second"}); err != nil {
		return assistant.PersonalChatReply{}, err
	}
	return assistant.PersonalChatReply{ConversationID: 42, ClientTurnID: in.ClientTurnID, State: "settled", Text: "final receipt"}, nil
}
func (p *assistantStreamHTTPFixture) WatchOutput(ctx context.Context, token string, id int64, turn string, consume func(assistant.AgentOutput) error) (assistant.PersonalChatReply, error) {
	return p.StreamChat(ctx, token, assistant.PersonalChatRequest{ConversationID: id, ClientTurnID: turn}, consume)
}
func (*assistantStreamHTTPFixture) Cancel(context.Context, string, int64, string) (assistant.PersonalChatReply, error) {
	return assistant.PersonalChatReply{ConversationID: 42, ClientTurnID: "turn", State: "unknown"}, nil
}

// Real HTTP/TCP and flush/backpressure, with controlled assistant/Core outcomes.
func TestAssistantSSEFlushesBeforeCompletionAndStopsOnDisconnect(t *testing.T) {
	for _, disconnect := range []bool{false, true} {
		t.Run(map[bool]string{false: "complete", true: "disconnect"}[disconnect], func(t *testing.T) {
			p := &assistantStreamHTTPFixture{release: make(chan struct{}), closed: make(chan struct{})}
			mux := http.NewServeMux()
			RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, p)
			server := httptest.NewServer(mux)
			defer server.Close()
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			defer cancel()
			req, _ := http.NewRequestWithContext(ctx, "POST", server.URL+"/api/assistant/chat", strings.NewReader(`{"message":"question","client_turn_id":"turn"}`))
			req.Header.Set("Authorization", "Bearer "+webToken)
			req.Header.Set("Accept", "text/event-stream")
			resp, err := http.DefaultClient.Do(req)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			if resp.StatusCode != 200 || !strings.Contains(resp.Header.Get("Content-Type"), "text/event-stream") || resp.Header.Get("X-Accel-Buffering") != "no" {
				t.Fatal(resp.Status, resp.Header)
			}
			reader := bufio.NewReader(resp.Body)
			var prefix strings.Builder
			for !strings.Contains(prefix.String(), "first prefix") {
				line, err := reader.ReadString('\n')
				if err != nil {
					t.Fatal(err, prefix.String())
				}
				prefix.WriteString(line)
			}
			if strings.Contains(prefix.String(), "event: done") {
				t.Fatal("buffered completion")
			}
			if disconnect {
				cancel()
				_ = resp.Body.Close()
			} else {
				close(p.release)
				rest, err := io.ReadAll(reader)
				if err != nil || !strings.Contains(string(rest), "first prefix plus second") || !strings.Contains(string(rest), "event: done") || !strings.Contains(string(rest), "final receipt") {
					t.Fatal(string(rest), err)
				}
			}
			select {
			case <-p.closed:
			case <-time.After(time.Second):
				t.Fatal("observer retained after disconnect")
			}
		})
	}
}

func TestAssistantSSECancelAndSnapshotCursorContract(t *testing.T) {
	p := &assistantStreamHTTPFixture{}
	mux := http.NewServeMux()
	RegisterAssistant(mux, newAuth(t, &authCore{}), &assistantPort{}, p)
	w := assistantRequest(mux, "POST", "/api/assistant/conversations/42/turns/turn/cancel", webToken, `{}`)
	if w.Code != 202 || !strings.Contains(w.Body.String(), `"state":"unknown"`) {
		t.Fatal(w.Code, w.Body.String())
	}
	w = assistantRequest(mux, "POST", "/api/assistant/conversations/42/turns/turn/cancel", webToken, `{"payer":1}`)
	if w.Code != 400 {
		t.Fatal(w.Code, w.Body.String())
	}
	req := httptest.NewRequest("GET", "/api/assistant/conversations/42/turns/turn/output", nil)
	req.Header.Set("Authorization", "Bearer "+webToken)
	req.Header.Set("Last-Event-ID", "1")
	w = httptest.NewRecorder()
	mux.ServeHTTP(w, req)
	if w.Code != 400 {
		t.Fatal(w.Code, w.Body.String())
	}
}
