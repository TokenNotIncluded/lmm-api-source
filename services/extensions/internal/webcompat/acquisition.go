package webcompat

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/acquisition"
	"net/http"
)

// RegisterAcquisition mounts the acquisition Web routes when the module is assembled.
func RegisterAcquisition(mux RouteRegistrar, auth *Authenticator, s *acquisition.Service) {
	if s != nil {
		s.Register(mux, auth)
	}
}

type acquisitionModule struct{ handler http.Handler }

func (*acquisitionModule) Name() string            { return "acquisition" }
func (m *acquisitionModule) Handler() http.Handler { return m.handler }

// NewAcquisitionModule exposes the same routes under /extensions/v1/acquisition for
// explicit host registration. The separate X-LMM-User-Credential header is the
// user credential; host admission is never user authority.
func NewAcquisitionModule(auth *Authenticator, s *acquisition.Service) modules.Module {
	mux := http.NewServeMux()
	s.Register(mux, auth)
	return &acquisitionModule{moduleUserCredential(mux)}
}
