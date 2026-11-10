// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type AssistantStream interface {
	StreamReady(context.Context) error
	StreamChat(context.Context, string, assistant.PersonalChatRequest, func(assistant.AgentOutput) error) (assistant.PersonalChatReply, error)
	WatchOutput(context.Context, string, int64, string, func(assistant.AgentOutput) error) (assistant.PersonalChatReply, error)
	Cancel(context.Context, string, int64, string) (assistant.PersonalChatReply, error)
}

func registerAssistantStreams(mux *http.ServeMux, chat AssistantChat, protected func(func(http.ResponseWriter, *http.Request, string)) http.HandlerFunc) {
	mux.HandleFunc("GET /api/assistant/conversations/{id}/turns/{turn}/output", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		stream, ok := chat.(AssistantStream)
		if !ok || access.Missing(stream) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		id, err := assistantConversationID(r)
		query, qerr := assistantQuery(r)
		// Core streams snapshots, not an event log. Reconnection uses the current
		// prefix and the durable turn receipt; it cannot honor an event cursor.
		if err != nil || qerr != nil || len(query) != 0 || r.Header.Get("Last-Event-ID") != "" {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		assistantSSE(w, r, func(consume func(assistant.AgentOutput) error) (assistant.PersonalChatReply, error) {
			return stream.WatchOutput(r.Context(), token, id, r.PathValue("turn"), consume)
		})
	}))
	mux.HandleFunc("POST /api/assistant/conversations/{id}/turns/{turn}/cancel", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		stream, ok := chat.(AssistantStream)
		if !ok || access.Missing(stream) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		id, err := assistantConversationID(r)
		query, qerr := assistantQuery(r)
		var body struct{}
		if err != nil || qerr != nil || len(query) != 0 || access.Body(w, r, &body) != nil {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		reply, err := stream.Cancel(r.Context(), token, id, r.PathValue("turn"))
		assistantChatRespond(w, reply, err)
	}))
}

func assistantSSE(w http.ResponseWriter, r *http.Request, run func(func(assistant.AgentOutput) error) (assistant.PersonalChatReply, error)) {
	if _, ok := w.(http.Flusher); !ok {
		assistantRespond(w, nil, access.ErrUnavailable)
		return
	}
	controller := http.NewResponseController(w)
	started := false
	send := func(name string, value any) error {
		if err := r.Context().Err(); err != nil {
			return err
		}
		data, err := json.Marshal(value)
		if err != nil {
			return err
		}
		if !started {
			w.Header().Set("Content-Type", "text/event-stream; charset=utf-8")
			w.Header().Set("Cache-Control", "no-store")
			w.Header().Set("X-Accel-Buffering", "no")
			w.Header().Set("X-Content-Type-Options", "nosniff")
			started = true
		}
		// Bound slow consumers without queueing prefixes or retaining an RPC slot.
		_ = controller.SetWriteDeadline(time.Now().Add(10 * time.Second))
		if _, err = fmt.Fprintf(w, "event: %s\ndata: %s\n\n", name, data); err != nil {
			return err
		}
		return controller.Flush()
	}
	defer func() {
		if started {
			_ = controller.SetWriteDeadline(time.Time{})
		}
	}()
	var round uint32
	seen := false
	reply, err := run(func(event assistant.AgentOutput) error {
		if !seen {
			if err := send("ready", map[string]any{"conversation_id": event.ConversationID, "client_turn_id": event.ClientTurnID}); err != nil {
				return err
			}
		}
		if !seen || event.Round != round {
			if err := send("replace", map[string]string{"content": ""}); err != nil {
				return err
			}
		}
		seen, round = true, event.Round
		switch event.Phase {
		case "model":
			return send("replace", map[string]string{"content": event.Text})
		case "tool":
			if err := send("replace", map[string]string{"content": ""}); err != nil {
				return err
			}
			return send("progress", map[string]any{"phase": "tool", "step": event.Round})
		default:
			return nil
		}
	})
	if err != nil {
		if !started {
			assistantRespond(w, nil, err)
			return
		}
		_ = send("error", map[string]any{"status": 503, "message": "Assistant output interrupted; inspect the original turn.", "retryable": false})
		return
	}
	if reply.State != "settled" || reply.Text == "" {
		if !started {
			assistantChatRespond(w, reply, nil)
			return
		}
		_ = send("error", map[string]any{"status": 202, "message": "Assistant operation is pending; inspect the original turn.", "retryable": false, "data": reply})
		return
	}
	_ = send("done", assistantChatSuccessBody(reply))
}
