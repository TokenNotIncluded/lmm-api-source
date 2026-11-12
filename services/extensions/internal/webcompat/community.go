// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"crypto/subtle"
	"net/http"
	"strings"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// VerifyAPIKey accepts one Bearer API key for client-reported progress. Core
// validates the key, its owner and revocation on every request; the key is
// never stored. Sessions and team-owned keys are rejected.
func (a *Authenticator) VerifyAPIKey(ctx context.Context, r *http.Request) (*pb.AuthorizeResponse, error) {
	if a == nil || store.Missing(a.core) {
		return nil, store.ErrUnavailable
	}
	if ctx == nil || r == nil {
		return nil, store.ErrUnauthorized
	}
	var values []string
	for name, entries := range r.Header {
		if strings.EqualFold(name, "Authorization") {
			values = append(values, entries...)
		}
	}
	if len(values) != 1 {
		return nil, store.ErrUnauthorized
	}
	token, ok := strings.CutPrefix(values[0], "Bearer ")
	if !ok || len(token) < 32 || len(token) > 256 || subtle.ConstantTimeCompare([]byte(token), a.hostToken) == 1 || strings.HasPrefix(token, "lmms_") {
		return nil, store.ErrUnauthorized
	}
	self, err := a.core.Authorize(ctx, token)
	if err != nil {
		return nil, authError(err)
	}
	if self == nil || self.GetUserId() <= 0 || self.GetPlatformLevel() < 0 || self.GetPlatformLevel() > 6 {
		return nil, store.ErrUnavailable
	}
	if self.GetCredentialKind() != pb.CredentialKind_CREDENTIAL_KIND_API_KEY ||
		self.GetOwner().GetKind() != pb.AccountKind_ACCOUNT_KIND_PERSONAL || self.GetOwner().GetId() != self.GetUserId() {
		return nil, store.ErrForbidden
	}
	return self, nil
}

// RegisterCommunity mounts the community Web routes when the module is assembled.
func RegisterCommunity(mux RouteRegistrar, auth *Authenticator, s *community.Service) {
	if s != nil {
		s.Register(mux, auth)
	}
}

type communityModule struct{ handler http.Handler }

func (*communityModule) Name() string            { return "community" }
func (m *communityModule) Handler() http.Handler { return m.handler }

// NewCommunityModule exposes the same routes under /extensions/v1/community for
// explicit host registration. The separate X-LMM-User-Credential header is the
// user credential; host admission is never user authority.
func NewCommunityModule(auth *Authenticator, s *community.Service) modules.Module {
	mux := http.NewServeMux()
	s.Register(mux, auth)
	return &communityModule{moduleUserCredential(mux)}
}
