package rustbridge

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

// These tests exercise request encoding and response interpretation in-process;
// the existing Unix-socket tests separately cover the actual socket transport.
type requestHTTPTransport struct{ http.Handler }

func (t requestHTTPTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	w := httptest.NewRecorder()
	t.ServeHTTP(w, r)
	return w.Result(), nil
}

func requestTestAdapter(handler http.Handler) *Adapter {
	return &Adapter{client: &http.Client{Transport: requestHTTPTransport{handler}}, token: strings.Repeat("h", 43)}
}

func TestRequestWireContracts(t *testing.T) {
	for _, tc := range []struct {
		name, protocol, action, user string
		requestBytes, responseBytes  int
		call                         func(*Adapter, any, any) error
	}{
		{"native", "1", "capabilities", "session", 16384, 65536, func(a *Adapter, in, out any) error {
			return a.request(context.Background(), "capabilities", "session", in, out)
		}},
		{"legacy", "2", "quotes", "session", 32768, 262144, func(a *Adapter, in, out any) error {
			return a.requestVersion(context.Background(), 2, "quotes", "session", in, out)
		}},
		{"terminal", "2", "calls/settle", "", 2850000, 65536, func(a *Adapter, in, out any) error {
			return a.requestLegacyResult(context.Background(), "calls/settle", in, out)
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			type response struct {
				code int
				body string
			}
			reply := response{http.StatusOK, "{}"}
			hits := 0
			var received string
			a := requestTestAdapter(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				hits++
				if r.Method != http.MethodPost || r.URL.Path != "/internal/tool-market/v"+tc.protocol+"/"+tc.action ||
					r.Header.Get("Authorization") != "Bearer "+strings.Repeat("h", 43) || r.Header.Get("X-Lmm-Protocol") != tc.protocol ||
					r.Header.Get("Content-Type") != "application/json" || r.Header.Get("X-Lmm-User-Credential") != tc.user {
					t.Error("request scope changed", r.Method, r.URL.Path, r.Header)
				}
				raw, err := io.ReadAll(r.Body)
				if err != nil {
					t.Fatal(err)
				}
				received = string(raw)
				w.WriteHeader(reply.code)
				_, _ = io.WriteString(w, reply.body)
			}))
			var out struct{}
			input := strings.Repeat("x", tc.requestBytes-2)
			if err := tc.call(a, input, &out); err != nil {
				t.Fatal("exact request limit rejected", err)
			}
			if received != `"`+input+`"` {
				t.Fatal("request body changed", len(received))
			}
			before := hits
			if err := tc.call(a, strings.Repeat("x", tc.requestBytes-1), &out); !errors.Is(err, market.ErrInvalid) || hits != before {
				t.Fatal("oversized request reached Core", err, hits-before)
			}
			for _, extra := range []int{0, 1} {
				reply = response{http.StatusOK, "{}" + strings.Repeat(" ", tc.responseBytes-2+extra)}
				err := tc.call(a, struct{}{}, &out)
				if extra == 0 && err != nil || extra == 1 && !errors.Is(err, market.ErrUnavailable) {
					t.Fatal("response limit changed", extra, err)
				}
			}
			for _, bad := range []string{`{"unknown":true}`, `{} {}`, `{`} {
				reply = response{http.StatusOK, bad}
				if err := tc.call(a, struct{}{}, &out); !errors.Is(err, market.ErrUnavailable) {
					t.Fatal("invalid response accepted", bad, err)
				}
			}
			for _, status := range []struct {
				code int
				err  error
			}{{400, market.ErrInvalid}, {401, market.ErrAuth}, {403, market.ErrDenied}, {404, market.ErrMissing}, {409, market.ErrConflict}, {503, market.ErrUnavailable}} {
				reply = response{status.code, "{}"}
				before := hits
				if err := tc.call(a, struct{}{}, &out); !errors.Is(err, status.err) || hits != before+1 {
					t.Fatal("status mapping or retry behavior changed", status.code, err, hits-before)
				}
			}
			before = hits
			if err := a.requestLegacyResult(context.Background(), "quotes", struct{}{}, &out); !errors.Is(err, market.ErrInvalid) || hits != before {
				t.Fatal("large-result scope expanded", err)
			}
		})
	}
}

func TestReportResponseLimit(t *testing.T) {
	const report = `{"rows":[]}`
	extra := 0
	a := requestTestAdapter(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/tool-market/v2/reports/list" || r.Header.Get("X-Lmm-User-Credential") != "session" {
			t.Error("report request changed", r.URL.Path, r.Header)
		}
		_, _ = io.Copy(io.Discard, r.Body)
		_, _ = io.WriteString(w, report+strings.Repeat(" ", (4<<20)-len(report)+extra))
	}))
	for _, n := range []int{0, 1} {
		t.Run(strconv.Itoa(n), func(t *testing.T) {
			extra = n
			_, err := a.ListLegacyReports(context.Background(), "session", market.LegacyReportListRequest{})
			if n == 0 && err != nil || n == 1 && !errors.Is(err, market.ErrUnavailable) {
				t.Fatal("report response limit changed", n, err)
			}
		})
	}
}
