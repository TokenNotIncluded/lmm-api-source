package composition

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc"
)

type assistantCapabilityServer struct {
	pb.UnimplementedCoreControlServer
	features []string
}

func (s *assistantCapabilityServer) Capabilities(context.Context, *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	return &pb.CapabilitiesResponse{ProtocolMajor: 1, Features: s.features}, nil
}

func TestAssistantMissingCredentialLeavesReservationAndModelUnavailable(t *testing.T) {
	r := &Runtime{}
	if err := r.composeAssistantDelegation(context.Background(), Config{}, nil, nil, nil); err != nil {
		t.Fatal(err)
	}
	if r.AssistantPending != nil || r.AssistantTurns != nil || r.AssistantModelClient != nil || r.AssistantChat != nil {
		t.Fatal("missing credential enabled an assistant port")
	}
	if token, err := loadAssistantCredential(Config{}, nil, nil, nil); err != nil || len(token) != 0 {
		t.Fatal("optional credential fell back to another identity")
	}
}

func TestRequiredAgentDoesNotFallBackToPersonalText(t *testing.T) {
	dir, err := os.MkdirTemp("/tmp", "lmm-agent-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	socket := filepath.Join(dir, "core.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	pb.RegisterCoreControlServer(server, &assistantCapabilityServer{features: []string{
		coreclient.AssistantPersonalTextReserveFeature,
		coreclient.AssistantPersonalTextExecuteFeature,
	}})
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(server.Stop)
	control, err := coreclient.New(socket, []byte(strings.Repeat("c", 32)))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = control.Close() })
	r := &Runtime{Client: control}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := r.composeAssistantDelegation(ctx, Config{Socket: socket, AssistantAgentRequired: true}, nil, nil, []byte(strings.Repeat("a", 32))); err == nil {
		t.Fatal("required agent fell back to a text-only Core")
	}
	if r.AssistantModelClient != nil || r.AssistantPending != nil || r.AssistantChat != nil {
		t.Fatal("unavailable agent left assistant ports exposed")
	}
	if err := (&Runtime{}).composeAssistantDelegation(ctx, Config{AssistantAgentRequired: true}, nil, nil, nil); err == nil {
		t.Fatal("required agent accepted a missing credential")
	}
}

func TestAssistantCredentialRejectsReuseOfEveryLoadedRole(t *testing.T) {
	roles := [][]byte{[]byte(strings.Repeat("h", 32)), []byte(strings.Repeat("i", 32)), []byte(strings.Repeat("c", 32)), []byte(strings.Repeat("p", 32))}
	for i, name := range []string{"host", "identity", "commerce", "payment"} {
		t.Run(name, func(t *testing.T) {
			file := filepath.Join(t.TempDir(), "assistant-token")
			if os.WriteFile(file, roles[i], 0o600) != nil {
				t.Fatal("cannot write private test credential")
			}
			cfg := Config{Selected: []string{"assistant"}, HostCredential: roles[0], AssistantTokenFile: file}
			if _, err := loadAssistantCredential(cfg, roles[1], roles[2], roles[3]); err == nil {
				t.Fatal("assistant credential reused a loaded role")
			}
		})
	}
	if err := validateAssistantSelection(Config{AssistantTokenFile: "/tmp/private-assistant-token"}); err == nil {
		t.Fatal("unselected assistant token was ignored")
	}
}
