package composition

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc"
)

type assistantCapabilityServer struct {
	pb.UnimplementedCoreControlServer
	features []string
	calls    atomic.Int64
}

func (s *assistantCapabilityServer) Capabilities(context.Context, *pb.CapabilitiesRequest) (*pb.CapabilitiesResponse, error) {
	s.calls.Add(1)
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

func assistantCapabilityFixture(t *testing.T, features []string) (string, *coreclient.Client, *assistantCapabilityServer) {
	t.Helper()
	socket := filepath.Join(t.TempDir(), "core.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer()
	capabilities := &assistantCapabilityServer{features: features}
	pb.RegisterCoreControlServer(server, capabilities)
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(server.Stop)
	control, err := coreclient.New(socket, []byte(strings.Repeat("c", 32)))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = control.Close() })
	return socket, control, capabilities
}

func TestRequiredAgentDoesNotFallBackToPersonalText(t *testing.T) {
	socket, control, capabilities := assistantCapabilityFixture(t, []string{
		coreclient.AssistantPersonalTextReserveFeature,
		coreclient.AssistantPersonalTextExecuteFeature,
	})
	r := &Runtime{Client: control}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := r.composeAssistantDelegation(ctx, Config{Socket: socket, AssistantAgentRequired: true}, nil, nil, []byte(strings.Repeat("a", 32))); err == nil {
		t.Fatal("required agent fell back to a text-only Core")
	}
	if got := capabilities.calls.Load(); got != 1 {
		t.Fatalf("required agent made %d capability calls; want only AgentReady", got)
	}
	if r.AssistantModelClient != nil || r.AssistantPending != nil || r.AssistantChat != nil {
		t.Fatal("unavailable agent left assistant ports exposed")
	}
	if err := (&Runtime{}).composeAssistantDelegation(ctx, Config{AssistantAgentRequired: true}, nil, nil, nil); err == nil {
		t.Fatal("required agent accepted a missing credential")
	}
}

func TestAssistantDisabledAgentDoesNotProbeOrOpenJournal(t *testing.T) {
	socket, control, capabilities := assistantCapabilityFixture(t, []string{coreclient.AssistantAgentTurnFeature})
	r := &Runtime{Client: control}
	defer r.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := r.composeAssistantDelegation(ctx, Config{Socket: socket, AssistantTokenFile: "configured"}, nil, nil, []byte(strings.Repeat("a", 32))); err != nil {
		t.Fatal("disabled agent tried to open a journal", err)
	}
	if got := capabilities.calls.Load(); got != 1 {
		t.Fatalf("disabled agent made %d capability calls; want only the PersonalText probe", got)
	}
	if r.AssistantModelClient != nil || r.AssistantPending != nil || r.AssistantChat != nil || r.AssistantServices != nil {
		t.Fatal("Core agent capability enabled a disabled assistant agent")
	}
}

func TestAssistantStorageBundleSelection(t *testing.T) {
	for _, test := range []struct {
		name     string
		required bool
		token    string
		bundle   storageBundle
		invalid  bool
	}{
		{"disabled without token", false, "", defaultStorageBundle, false},
		{"personal text", false, "configured", assistantDelegationBundle, false},
		{"required with token", true, "configured", assistantAgentBundle, false},
		{"required without token", true, "", assistantAgentBundle, true},
	} {
		t.Run(test.name, func(t *testing.T) {
			cfg := Config{Selected: []string{"assistant"}, AssistantTokenFile: test.token, AssistantAgentRequired: test.required}
			if got := assistantStorageBundle(cfg); got != test.bundle {
				t.Fatalf("storage bundle = %v, want %v", got, test.bundle)
			}
			if err := validateAssistantSelection(cfg); (err != nil) != test.invalid {
				t.Fatalf("assistant configuration validation = %v", err)
			}
		})
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
