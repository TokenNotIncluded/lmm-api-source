// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
	"strings"
)

// Derived code records are overlaid on a transaction's private copy. Opening or
// reading a module does not initialize/migrate its database. A code/schema change
// points to a new immutable version, so prior grants cannot authorize it.
func overlayBuiltinCatalog(d *database) {
	w := webState(d)
	for _, definition := range CompiledBuiltinDefinitions() {
		tools := make([]WebTool, 0, len(definition.Tools))
		remote := map[string]string{}
		for _, entry := range definition.Tools {
			if definition.Key == "open_source_bounties" && !bounties.AvailableAction(strings.TrimPrefix(entry.Name, "open_source_bounties.")) {
				continue
			}
			tool := entry.WebTool
			tool.PriceQuota = 0
			tool.ProviderPricing = nil
			tool.BillingMode = ""
			tool.BillingRules = nil
			tool.InputTokenPriceQuota = 0
			tool.MaxInputTokens = 0
			tool.VersionID = definition.VersionID
			tools = append(tools, tool)
			remote[tool.ToolID] = entry.InputSchemaDigest
		}
		if definition.Key == "open_source_bounties" {
			definition.Name = "Historical bounty settlements"
			definition.Description = "Review, settle or dispute existing obligations with the original account."
		}
		w.Services[definition.ServiceID] = WebServiceRecord{ID: definition.ServiceID, OwnerID: 0, LiveVersionID: definition.VersionID, Status: "published"}
		w.Versions[definition.VersionID] = webSnapshot{Version: WebVersion{ID: definition.VersionID, ServiceID: definition.ServiceID, Status: "published", Name: definition.Name, Description: definition.Description, ExecutionType: "builtin", Visibility: "public", Digest: definition.Digest}, Tools: tools, Digest: definition.Digest, ValidationDigest: definition.Digest, RemoteDigests: remote}
	}
}

func (m *Module) builtinVersionAvailable(v webSnapshot) bool {
	if v.Version.ExecutionType != "builtin" {
		return true
	}
	if missingInterface(m.builtinRuntime) {
		return false
	}
	for _, definition := range CompiledBuiltinDefinitions() {
		if definition.ServiceID == v.Version.ServiceID && definition.VersionID == v.Version.ID && definition.Digest == v.Digest {
			return m.builtinRuntime.BuiltinServiceAvailable(definition.Key)
		}
	}
	return false
}
