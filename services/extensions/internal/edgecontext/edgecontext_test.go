package edgecontext

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestCaptureStripsHeadersAndRequiresSingleLoopbackCarrier(t *testing.T) {
	for _, tc := range []struct {
		name, remote, ip string
		proof            []string
		want             bool
	}{
		{"valid", "127.0.0.1:1000", "::ffff:203.0.113.7", []string{strings.Repeat("p", 48)}, true},
		{"external", "192.0.2.1:1000", "203.0.113.7", []string{strings.Repeat("p", 48)}, false},
		{"duplicate", "127.0.0.1:1000", "203.0.113.7", []string{strings.Repeat("p", 48), strings.Repeat("p", 48)}, false},
		{"missing", "127.0.0.1:1000", "203.0.113.7", nil, false},
		{"chain", "127.0.0.1:1000", "203.0.113.7,127.0.0.1", []string{strings.Repeat("p", 48)}, false},
		{"port", "127.0.0.1:1000", "203.0.113.7:80", []string{strings.Repeat("p", 48)}, false},
		{"zone", "127.0.0.1:1000", "fe80::1%lo", []string{strings.Repeat("p", 48)}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := httptest.NewRequest("POST", "/api/assistant/new-user-gift/claim", nil)
			r.RemoteAddr = tc.remote
			r.Header[ProofHeader] = tc.proof
			r.Header.Set(ClientHeader, tc.ip)
			h := Capture(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get(ProofHeader) != "" || r.Header.Get(ClientHeader) != "" {
					t.Fatal("private header reached handler")
				}
				proof, ip, ok := Assertion(r.Context())
				if ok != tc.want || (ok && (proof != tc.proof[0] || ip != "203.0.113.7")) {
					t.Fatal("wrong carrier presence or normalized address")
				}
			}))
			h.ServeHTTP(httptest.NewRecorder(), r)
		})
	}
}
