package rustauth

import (
	"context"
	"errors"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func boundSession() *pb.AuthorizeResponse {
	value := session()
	value.CredentialId = 10
	value.SessionBinding = "v1:" + strings.Repeat("a", 64)
	return value
}

func TestSessionBindingUsesFreshCoreAuthority(t *testing.T) {
	core := &coreProbe{self: boundSession()}
	adapter, _ := New(core)
	first, err := adapter.SessionBinding(context.Background(), "old-access", 1)
	if err != nil {
		t.Fatal(err)
	}
	core.self.CredentialId = 11
	refreshed, err := adapter.SessionBinding(context.Background(), "new-access", 1)
	if err != nil || refreshed != first || core.calls != 2 {
		t.Fatalf("refresh binding lost or cached: %q %v calls=%d", refreshed, err, core.calls)
	}
	core.self.SessionBinding = "v1:" + strings.Repeat("b", 64)
	other, err := adapter.SessionBinding(context.Background(), "other-login", 1)
	if err != nil || other == first {
		t.Fatal("another login collapsed to user identity", err)
	}
	core.err = status.Error(codes.Unauthenticated, "revoked")
	if _, err := adapter.SessionBinding(context.Background(), "new-access", 1); !errors.Is(err, store.ErrUnauthorized) {
		t.Fatal("revocation was cached", err)
	}
}

func TestSessionBindingRejectsMissingContractAndUntrustedAuthority(t *testing.T) {
	for _, test := range []struct {
		name   string
		mutate func(*pb.AuthorizeResponse)
		user   int64
		want   error
	}{
		{"old-core", func(v *pb.AuthorizeResponse) { v.SessionBinding = "" }, 1, store.ErrUnavailable},
		{"unknown-version", func(v *pb.AuthorizeResponse) { v.SessionBinding = "v2:" + strings.Repeat("a", 64) }, 1, store.ErrUnavailable},
		{"malformed", func(v *pb.AuthorizeResponse) { v.SessionBinding = "v1:" + strings.Repeat("z", 64) }, 1, store.ErrUnavailable},
		{"api-key", func(v *pb.AuthorizeResponse) { v.CredentialKind = pb.CredentialKind_CREDENTIAL_KIND_API_KEY }, 1, store.ErrForbidden},
		{"wrong-user", func(*pb.AuthorizeResponse) {}, 2, store.ErrForbidden},
		{"wrong-owner", func(v *pb.AuthorizeResponse) { v.Owner.Id = 2 }, 1, store.ErrForbidden},
		{"missing-credential", func(v *pb.AuthorizeResponse) { v.CredentialId = 0 }, 1, store.ErrUnauthorized},
	} {
		t.Run(test.name, func(t *testing.T) {
			core := &coreProbe{self: boundSession()}
			test.mutate(core.self)
			adapter, _ := New(core)
			if _, err := adapter.SessionBinding(context.Background(), "access", test.user); !errors.Is(err, test.want) {
				t.Fatalf("want %v, got %v", test.want, err)
			}
			// The additive contract must not disable ordinary personal checks on old Core.
			if test.name == "old-core" {
				if id, err := adapter.Check(context.Background(), "access", store.Account{Kind: "personal", ID: 1}, store.Manage); err != nil || id != 1 {
					t.Fatal("old Authority.Check contract changed", id, err)
				}
			}
		})
	}
}
