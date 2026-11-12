// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type AssistantConversationLookup interface {
	LookupChatConversation(context.Context, string, string) (assistant.ChatConversationMapping, error)
}

type AssistantConversationRecovery interface {
	RecoverChatConversation(context.Context, string, string) (assistant.ChatConversationMapping, error)
}

func registerAssistantConversationLookup(mux RouteRegistrar, chat AssistantChat, protected func(func(http.ResponseWriter, *http.Request, string)) http.HandlerFunc) {
	mux.HandleFunc("GET /api/assistant/turns/{turn}/conversation", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		lookup, ok := chat.(AssistantConversationLookup)
		if !ok || access.Missing(lookup) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		query, err := assistantQuery(r)
		if err != nil || len(query) != 0 || r.ContentLength != 0 {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		mapping, err := lookup.LookupChatConversation(r.Context(), token, r.PathValue("turn"))
		if err != nil || mapping.Phase != "creation_inflight" {
			assistantRespond(w, mapping, err)
			return
		}
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.WriteHeader(http.StatusAccepted)
		_ = json.NewEncoder(w).Encode(map[string]any{"success": true, "message": "", "data": mapping})
	}))
	mux.HandleFunc("POST /api/assistant/turns/{turn}/conversation/recover", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		recovery, ok := chat.(AssistantConversationRecovery)
		if !ok || access.Missing(recovery) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		query, err := assistantQuery(r)
		var body struct{}
		if err != nil || len(query) != 0 || access.Body(w, r, &body) != nil {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		mapping, err := recovery.RecoverChatConversation(r.Context(), token, r.PathValue("turn"))
		if !errors.Is(err, assistant.ErrDelegationUnknown) {
			assistantRespond(w, mapping, err)
			return
		}
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.WriteHeader(http.StatusAccepted)
		_ = json.NewEncoder(w).Encode(map[string]any{"success": false, "code": "ASSISTANT_CONVERSATION_RECOVERY_UNKNOWN", "message": "Original conversation receipt is not confirmed.", "data": mapping})
	}))

}
