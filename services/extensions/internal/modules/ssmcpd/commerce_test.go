package ssmcpd

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestPaidOrderGrantRequiresExactDaemonAcknowledgement(t *testing.T) {
	original := grantRequest{"17", "order-7", "2026-11", "8000000000"}
	good := grantReply{original, "granted"}
	wrong := good
	wrong.Bytes = "8000000001"
	for _, tc := range []struct {
		name string
		code int
		body any
		want string
	}{
		{"granted", 200, good, "granted"}, {"original replay", 200, grantReply{original, "replayed"}, "granted"}, {"changed bytes", 200, wrong, ""}, {"missing fields", 200, map[string]string{"status": "granted"}, ""}, {"unrecognized state", 200, grantReply{original, "paid"}, ""},
		{"expired new order", 409, map[string]string{"error": "period_mismatch"}, "rejected"}, {"unknown conflict", 409, map[string]string{"error": "other"}, ""}, {"unbound after dispatch", 404, map[string]string{"error": "unknown_user"}, ""}, {"unavailable", 503, nil, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Method != "POST" || r.URL.Path != "/internal/v1/ssmcp/entitlements" || r.Header.Get("Authorization") != "Bearer "+strings.Repeat("a", 64) || r.Header.Get("X-LMM-User-Credential") != "" {
					t.Error("unexpected grant transport")
				}
				var sent grantRequest
				if json.NewDecoder(r.Body).Decode(&sent) != nil || sent != original {
					t.Error("changed original grant")
				}
				w.WriteHeader(tc.code)
				_ = json.NewEncoder(w).Encode(tc.body)
			}))
			defer srv.Close()
			client := internalClient{http: srv.Client(), origin: srv.URL, token: strings.Repeat("a", 64)}
			if got := client.issue(context.Background(), original); got != tc.want {
				t.Fatalf("got %q, want %q", got, tc.want)
			}
		})
	}
}
