package checkins

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestHTTPRejectsAmbiguityBeforeAuthorityOrStorage(t *testing.T) {
	s := &Service{}
	for _, tc := range []struct {
		name, body, query string
		headers           []string
		want              int
	}{
		{"missing user", `{"operation_key":"a-valid-operation-key"}`, "", nil, 401},
		{"duplicate user", `{"operation_key":"a-valid-operation-key"}`, "", []string{"Bearer first", "Bearer second"}, 401},
		{"duplicate field", `{"operation_key":"a-valid-operation-key","operation_key":"another-operation-key"}`, "", []string{"Bearer session"}, 400},
		{"unknown field", `{"operation_key":"a-valid-operation-key","amount":100}`, "", []string{"Bearer session"}, 400},
		{"trailing value", `{"operation_key":"a-valid-operation-key"}{}`, "", []string{"Bearer session"}, 400},
		{"malformed query", `{}`, "?month=%zz", []string{"Bearer session"}, 400},
		{"unverified turnstile", `{"operation_key":"a-valid-operation-key"}`, "?turnstile=unverified", []string{"Bearer session"}, 503},
	} {
		t.Run(tc.name, func(t *testing.T) {
			mux := http.NewServeMux()
			s.Register(mux)
			request := httptest.NewRequest(http.MethodPost, "/api/user/checkin"+tc.query, strings.NewReader(tc.body))
			for _, v := range tc.headers {
				request.Header.Add("Authorization", v)
			}
			response := httptest.NewRecorder()
			mux.ServeHTTP(response, request)
			if response.Code != tc.want {
				t.Fatalf("%d %s", response.Code, response.Body.String())
			}
			if !strings.Contains(response.Header().Get("Cache-Control"), "no-store") {
				t.Fatal("authentication/receipt response cacheable")
			}
		})
	}
}
