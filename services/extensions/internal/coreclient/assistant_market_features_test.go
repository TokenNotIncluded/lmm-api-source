package coreclient

import (
	"strings"
	"testing"
)

func TestAgentMarketToolsRequireImplementedCoreFeature(t *testing.T) {
	for name, required := range map[string]string{"get_connected_market_tools": assistantMarketCatalogueFeature, "call_market_tool": assistantMarketExecutionFeature} {
		t.Run(name, func(t *testing.T) {
			b := AssistantExtensionBinding{Name: name, Version: "core-market-authority-v1", SchemaSHA256: strings.Repeat("a", 64), PriceVersion: "builtin-free-v1", PriceSHA256: strings.Repeat("b", 64)}
			tuple := extensionTuple(b)
			features := []string{AssistantAgentTurnFeature, "assistant.agent_extension.v1:" + name + ":" + extensionDigest(tuple), "assistant.agent_extensions.sha256:" + extensionDigest([][5]string{tuple})}
			if validateAgentExtensions(features, []AssistantExtensionBinding{b}) == nil {
				t.Fatal("registry hash alone advertised missing Core runtime")
			}
			if err := validateAgentExtensions(append(features, required), []AssistantExtensionBinding{b}); err != nil {
				t.Fatal(err)
			}
			b.Version = "main-3948a0a1"
			if validateAgentExtensions(append(features, required), []AssistantExtensionBinding{b}) == nil {
				t.Fatal("old generic reserve adapter negotiated new authority")
			}
		})
	}
}
