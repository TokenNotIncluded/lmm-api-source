package composition

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

type checkinWireCore struct {
	pb.UnimplementedCoreCheckinLedgerServer
	t     *testing.T
	token string
	calls atomic.Int64
}

func (c *checkinWireCore) Inspect(ctx context.Context, _ *pb.CheckinLedgerInspect) (*pb.CheckinLedgerContext, error) {
	m, _ := metadata.FromIncomingContext(ctx)
	if v := m.Get("authorization"); len(v) != 1 || v[0] != "Bearer "+c.token {
		return nil, status.Error(codes.Unauthenticated, "wrong service authority")
	}
	if v := m.Get("x-lmm-user-credential"); len(v) != 1 || v[0] != strings.Repeat("u", 48) {
		return nil, status.Error(codes.Unauthenticated, "missing user authority")
	}
	if v := m.Get("x-lmm-protocol"); len(v) != 1 || v[0] != "1" {
		return nil, status.Error(codes.FailedPrecondition, "wrong protocol")
	}
	c.calls.Add(1)
	at := time.Now().Unix()
	return &pb.CheckinLedgerContext{UserId: 17, AccountId: 29, Timestamp: at, Day: time.Unix(at+28800, 0).UTC().Format("2006-01-02"), TrustLevel: 4, MaxDailyUnits: 100}, nil
}
func TestCheckinsModuleComposesStorageHTTPAndDedicatedRPC(t *testing.T) {
	path := os.Getenv("LMM_CHECKINS_TEST_RUNTIME_URL_FILE")
	if path == "" {
		t.Skip("requires explicit synthetic checkins storage")
	}
	dir, e := os.MkdirTemp("", "lmm-checkin-wire-")
	if e != nil {
		t.Fatal(e)
	}
	defer os.RemoveAll(dir)
	socket := filepath.Join(dir, "core.sock")
	listener, e := net.Listen("unix", socket)
	if e != nil {
		t.Fatal(e)
	}
	server := grpc.NewServer()
	wire := &checkinWireCore{t: t, token: strings.Repeat("f", 48)}
	pb.RegisterCoreCheckinLedgerServer(server, wire)
	go server.Serve(listener)
	defer server.Stop()
	identityFile := filepath.Join(dir, "identity")
	fundingFile := filepath.Join(dir, "funding")
	os.WriteFile(identityFile, []byte(strings.Repeat("i", 48)), 0600)
	os.WriteFile(fundingFile, []byte(wire.token), 0600)
	hostToken := []byte(strings.Repeat("h", 48))
	cfg := Config{Selected: []string{"checkins"}, Socket: socket, TokenFile: identityFile, CheckinTokenFile: fundingFile, HostCredential: hostToken, Checkins: StorageConfig{Component: "checkins", URLFile: path, Database: "lmm_checkins_request_test"}}
	runtime, e := New(context.Background(), cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer runtime.Close()
	if len(runtime.Modules) != 1 || runtime.Checkins == nil {
		t.Fatal("checkins not assembled")
	}
	host, e := modules.New(runtime.Modules, hostToken)
	if e != nil {
		t.Fatal(e)
	}
	request := httptest.NewRequest(http.MethodGet, "/extensions/v1/checkins/api/user/checkin", nil)
	request.Header.Set("Authorization", "Bearer "+string(hostToken))
	request.Header.Set("X-LMM-User-Credential", strings.Repeat("u", 48))
	response := httptest.NewRecorder()
	host.ServeHTTP(response, request)
	if response.Code != 200 || wire.calls.Load() != 1 || !strings.Contains(response.Body.String(), `"records":[]`) {
		t.Fatalf("module status %d %s calls=%d", response.Code, response.Body.String(), wire.calls.Load())
	}
	request = httptest.NewRequest(http.MethodGet, "/extensions/v1/checkins/api/user/checkin", nil)
	request.Header.Set("Authorization", "Bearer "+string(hostToken))
	response = httptest.NewRecorder()
	host.ServeHTTP(response, request)
	if response.Code != 401 || wire.calls.Load() != 1 {
		t.Fatal("host credential upgraded to user authority")
	}
	web := http.NewServeMux()
	runtime.Checkins.Register(web)
	anonymous := httptest.NewRecorder()
	web.ServeHTTP(anonymous, httptest.NewRequest(http.MethodGet, "/api/checkin/capabilities", nil))
	if anonymous.Code != 200 || !strings.Contains(anonymous.Body.String(), `"enabled":`) || wire.calls.Load() != 1 {
		t.Fatalf("anonymous policy capability %d %s", anonymous.Code, anonymous.Body.String())
	}
	cfg.CheckinTokenFile = identityFile
	if r, e := New(context.Background(), cfg); e == nil {
		r.Close()
		t.Fatal("funding credential aliases identity authority")
	}
}
