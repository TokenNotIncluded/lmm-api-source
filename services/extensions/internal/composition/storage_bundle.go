// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"errors"
	"slices"
)

type storageBundle uint8

const (
	defaultStorageBundle storageBundle = iota
	toolMarketBuiltinsBundle
	assistantDelegationBundle
	assistantAgentBundle
)

func assistantStorageBundle(cfg Config) storageBundle {
	if cfg.AssistantAgentRequired {
		return assistantAgentBundle
	}
	if cfg.AssistantTokenFile != "" {
		return assistantDelegationBundle
	}
	return defaultStorageBundle
}

type storageContract struct {
	name    string
	version int
}

type storageColumnGrant struct {
	name    string
	columns []string
}

type storagePolicy struct {
	namespaces       []string
	tables           []storageTable
	forbidden        []storageTable
	forbiddenColumns []storageTable
	contracts        []storageContract
	updateColumns    []storageColumnGrant
	sequences        []string
}

func storagePolicyFor(component string, bundle storageBundle) (storagePolicy, error) {
	switch component {
	case "store", "support", "assistant", "payments", "operations", "toolmarket":
	default:
		return storagePolicy{}, errors.New("unknown extension storage component")
	}
	if bundle != defaultStorageBundle &&
		!(bundle == toolMarketBuiltinsBundle && component == "toolmarket") &&
		!((bundle == assistantDelegationBundle || bundle == assistantAgentBundle) && component == "assistant") {
		return storagePolicy{}, errors.New("invalid extension storage bundle")
	}
	policy := storagePolicy{namespaces: []string{"lmm_" + component}, tables: storageTables(component)}
	if component == "assistant" {
		policy.contracts = []storageContract{{"lmm_assistant.contract", 2}}
		policy.updateColumns = []storageColumnGrant{{"lmm_assistant.preset_stats", []string{"clicks"}}}
		policy.sequences = []string{"lmm_assistant.conversations_id_seq", "lmm_assistant.history_messages_id_seq"}
		if bundle == assistantDelegationBundle || bundle == assistantAgentBundle {
			policy.tables = append(policy.tables,
				storageTable{"lmm_assistant.delegation_contract", []string{"SELECT"}},
				storageTable{"lmm_assistant.delegation_conversations", []string{"SELECT", "INSERT"}},
				storageTable{"lmm_assistant.delegation_turns", []string{"SELECT", "INSERT"}},
			)
			policy.contracts = append(policy.contracts, storageContract{"lmm_assistant.delegation_contract", 1})
			policy.updateColumns = append(policy.updateColumns,
				storageColumnGrant{"lmm_assistant.delegation_conversations", []string{"core_conversation_id", "phase"}},
				storageColumnGrant{"lmm_assistant.delegation_turns", []string{"body", "revision"}},
			)
		}
		if bundle == assistantAgentBundle {
			policy.tables = append(policy.tables,
				storageTable{"lmm_assistant.agent_contract", []string{"SELECT"}},
				storageTable{"lmm_assistant.agent_turns", []string{"SELECT", "INSERT"}},
				storageTable{"lmm_assistant.services_contract", []string{"SELECT"}},
			)
			policy.contracts = append(policy.contracts,
				storageContract{"lmm_assistant.agent_contract", 1},
				storageContract{"lmm_assistant.services_contract", 1},
			)
			policy.updateColumns = append(policy.updateColumns,
				storageColumnGrant{"lmm_assistant.agent_turns", []string{"body", "body_bytes", "revision"}},
				storageColumnGrant{"lmm_assistant.service_confirmations", []string{"consumed_at", "result"}},
			)
			// AgentChat always composes assistant_services and personalization.
			for _, name := range []string{"memories", "site_issues", "site_issue_events", "platform_support", "platform_support_messages"} {
				policy.sequences = append(policy.sequences, "lmm_assistant."+name+"_id_seq")
			}
			for _, name := range []string{"memories", "profiles"} {
				policy.tables = append(policy.tables, storageTable{"lmm_assistant." + name, []string{"SELECT", "INSERT", "UPDATE", "DELETE"}})
			}
			for _, name := range []string{"site_issues", "platform_support", "platform_support_todos"} {
				policy.tables = append(policy.tables, storageTable{"lmm_assistant." + name, []string{"SELECT", "INSERT", "UPDATE"}})
			}
			for _, name := range []string{"service_operations", "service_network_dispatch", "service_confirmations", "site_issue_events", "platform_support_messages"} {
				policy.tables = append(policy.tables, storageTable{"lmm_assistant." + name, []string{"SELECT", "INSERT"}})
			}
		}
		for _, table := range policy.tables {
			for _, privilege := range []string{"SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"} {
				if slices.Contains(table.privileges, privilege) {
					continue
				}
				policy.forbidden = append(policy.forbidden, storageTable{table.name, []string{privilege}})
				columnUpdate := privilege == "UPDATE" && slices.ContainsFunc(policy.updateColumns, func(grant storageColumnGrant) bool { return grant.name == table.name })
				if !columnUpdate && (privilege == "SELECT" || privilege == "INSERT" || privilege == "UPDATE" || privilege == "REFERENCES") {
					policy.forbiddenColumns = append(policy.forbiddenColumns, storageTable{table.name, []string{privilege}})
				}
			}
		}
	}
	if bundle == toolMarketBuiltinsBundle {
		policy.namespaces = append(policy.namespaces, "lmm_bounties", "extension_wallet")
		policy.tables = append(policy.tables,
			storageTable{"lmm_bounties.contract", []string{"SELECT"}},
			storageTable{"lmm_bounties.records", []string{"SELECT", "INSERT", "UPDATE"}},
			storageTable{"extension_wallet.transfer_links", []string{"SELECT", "INSERT"}},
		)
		policy.forbidden = []storageTable{
			{"lmm_toolmarket.snapshot", []string{"INSERT", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"lmm_bounties.contract", []string{"INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"lmm_bounties.records", []string{"DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
			{"extension_wallet.transfer_links", []string{"UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN"}},
		}
		policy.forbiddenColumns = []storageTable{
			{"lmm_toolmarket.snapshot", []string{"INSERT", "REFERENCES"}},
			{"lmm_bounties.contract", []string{"INSERT", "UPDATE", "REFERENCES"}},
			{"lmm_bounties.records", []string{"REFERENCES"}},
			{"extension_wallet.transfer_links", []string{"UPDATE", "REFERENCES"}},
		}
	}
	return policy, nil
}
