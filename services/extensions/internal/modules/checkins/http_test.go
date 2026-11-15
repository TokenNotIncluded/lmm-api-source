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
		{"duplicate proof", `{"operation_key":"a-valid-operation-key","challenge_id":"one","challenge_id":"two"}`, "", []string{"Bearer session"}, 400},
		{"invalid assertion", `{"operation_key":"a-valid-operation-key","passkey_assertion":true}`, "", []string{"Bearer session"}, 400},
		{"null assertion", `{"operation_key":"a-valid-operation-key","passkey_assertion":null}`, "", []string{"Bearer session"}, 400},
		{"trailing value", `{"operation_key":"a-valid-operation-key"}{}`, "", []string{"Bearer session"}, 400},
		{"malformed query", `{}`, "?month=%zz", []string{"Bearer session"}, 400},
		{"unverified turnstile", `{"operation_key":"a-valid-operation-key"}`, "?turnstile=unverified", []string{"Bearer session"}, 400},
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

func TestCheckinProofBodyAndChallengeBoundary(t *testing.T) {
	body := `{"operation_key":"a-valid-operation-key","challenge_id":"challenge-value","turnstile_token":"proof-value","passkey_assertion":{"id":"credential","response":{"signature":"sig"}}}`
	request := httptest.NewRequest(http.MethodPost, "/api/user/checkin", strings.NewReader(body))
	input, e := readClaimRequest(httptest.NewRecorder(), request, false)
	if e != nil || input.ChallengeID != "challenge-value" || input.TurnstileToken != "proof-value" || !strings.Contains(string(input.PasskeyAssertion), `"signature":"sig"`) {
		t.Fatalf("proof lost from body: %+v %v", input, e)
	}
	request = httptest.NewRequest(http.MethodPost, "/api/user/checkin/challenges", strings.NewReader(body))
	if _, e := readClaimRequest(httptest.NewRecorder(), request, true); e != ErrInvalid {
		t.Fatal("challenge creation accepts supplied verification policy or proof")
	}
	for _, tc := range []struct {
		err    error
		status int
		code   string
	}{
		{ErrVerificationRequired, 403, "CHECKIN_VERIFICATION_REQUIRED"},
		{ErrVerificationFailed, 403, "CHECKIN_VERIFICATION_FAILED"},
		{ErrPasskeyRequired, 409, "CHECKIN_PASSKEY_REQUIRED"},
		{ErrRateLimited, 429, "CHECKIN_RATE_LIMITED"},
	} {
		r := httptest.NewRecorder()
		write(r, nil, tc.err)
		if r.Code != tc.status || !strings.Contains(r.Body.String(), tc.code) {
			t.Fatalf("error contract: %d %s", r.Code, r.Body.String())
		}
		if tc.err == ErrRateLimited && r.Header().Get("Retry-After") != "60" {
			t.Fatal("rate limit omitted retry guidance")
		}
	}
}
