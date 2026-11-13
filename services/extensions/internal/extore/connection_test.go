package extore

import (
	"context"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestRevokeConnectionPinsFormAndHidesToken(t *testing.T) {
	credentials := Credentials{Access: "access-secret", Refresh: "refresh-secret", GrantID: "grant"}
	calls := 0
	c := Client{HTTP: &http.Client{Transport: roundTrip(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.Method != http.MethodPost || r.URL.String() != "https://extore.example"+revokePath {
			t.Fatalf("unexpected revoke request %s %s", r.Method, r.URL)
		}
		if r.Header.Get("Authorization") != "" {
			t.Fatal("revoke must not send a bearer token")
		}
		if err := r.ParseForm(); err != nil {
			t.Fatal(err)
		}
		if r.Form.Get("token") != "refresh-secret" || r.Form.Get("client_id") != "client" || r.Form.Get("token_type_hint") != "refresh_token" {
			t.Fatalf("wrong revoke form %v", r.Form)
		}
		return reply(nil), nil
	})}}
	if err := c.RevokeConnection(context.Background(), "https://extore.example", "client", credentials); err != nil {
		t.Fatal(err)
	}
	if calls != 1 {
		t.Fatal(calls)
	}
}

func TestRevokeConnectionFailsClosed(t *testing.T) {
	credentials := Credentials{Refresh: "refresh-secret"}
	for name, failure := range map[string]func() (*http.Response, error){
		"status": func() (*http.Response, error) {
			return &http.Response{StatusCode: 400, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{"error":"invalid_token"}`))}, nil
		},
		"transport": func() (*http.Response, error) {
			return nil, errors.New("dial tcp: upstream secret detail")
		},
	} {
		t.Run(name, func(t *testing.T) {
			c := Client{HTTP: &http.Client{Transport: roundTrip(func(*http.Request) (*http.Response, error) { return failure() })}}
			err := c.RevokeConnection(context.Background(), "https://extore.example", "client", credentials)
			if !errors.Is(err, ErrUnavailable) {
				t.Fatalf("want safe failure, got %v", err)
			}
			if strings.Contains(err.Error(), "refresh-secret") {
				t.Fatal("token leaked into error")
			}
		})
	}
	if err := (Client{}).RevokeConnection(context.Background(), "https://extore.example", "client", Credentials{}); !errors.Is(err, ErrInput) {
		t.Fatal("empty refresh token accepted")
	}
}
