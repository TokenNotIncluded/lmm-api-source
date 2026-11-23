package composition

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_catalog"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
)

func (r *Runtime) assistantCatalogueBindings(authority access.Authority) ([]assistant.AgentToolBinding, error) {
	var bounty *bounties.Service
	if r.Builtins != nil {
		bounty = r.Builtins.Bounties
	}
	reader, err := assistant_catalog.New(authority, r.Store, bounty)
	if err != nil {
		return nil, err
	}
	return reader.Bindings(), nil
}
