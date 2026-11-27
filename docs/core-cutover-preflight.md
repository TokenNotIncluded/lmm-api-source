# Core cutover preflight

`scripts/core-cutover-preflight.py` is a read-only gate for a production cutover
record. It does not connect to PostgreSQL, SSH, systemd, Nginx or a provider, and
it never reads a credential file. The input is a sanitized contract record: it
contains capability booleans, enum values and SHA-256 bindings only. Duplicate
JSON fields, unknown fields, literal secret fields and an unbound contract are
rejected before a gate result is produced.

A blocked result exits with status `1`; malformed or stale evidence exits with
status `2`. The observation timestamp must be timezone-aware and no more than
15 minutes old. `READY` with `safe_to_cutover=true` means only that the trusted
input assertions are consistent with this preflight's rules; it is not a
production cutover approval. The preflight does not read or independently
verify the evidence files behind the assertions or hashes. Actual runtime,
backup and restore, financial, and provider validation remain required, along
with independent acceptance. A healthy Core process alone is insufficient.

The preflight also requires a `release` provenance block. It binds the exact
40-character source commit to the Core and Extensions artifact digests, SBOM,
schema fingerprint, provider/identity/financial/edge evidence digests and the
runtime release identity. Runtime source and artifact digests must match the
candidate package, and the writer epoch must advance beyond the previous epoch.
This is still an evidence contract: a correctly shaped hash is not proof that a
provider, database or public edge actually produced it. Independent host and
runtime checks must calculate these values and bind them to the same record.

Core's public `/api/status` response exposes only the product version
(`X-LMM-API-Version`) and a request ID. Source commits, artifact digests,
schema fingerprints and route identities are private cutover evidence and are
never returned by the public status endpoint. The preflight binds those values
from the candidate manifest, package readback and runtime evidence instead;
the hashes alone are not proof of a provider, database or public edge check.

Current rehearsal state and remaining cutover blockers are maintained in the
[architecture progress](progress.md). Dated WIP contracts and host snapshots
are retained only as fixed-version evidence in the [history index](archive.md);
they are not current readiness claims or cutover approval.

## Missing contracts and credentials

The validator reports the following as separate gates. A schema catalog or
`/health/ready: 200` does not satisfy any of them.

| Gate | Required evidence before enablement |
| --- | --- |
| Model forwarding | Core model file with non-empty routes, enabled upstream/target bindings, exact credential-reference set and provider-context contract; private upstream credentials; a non-billable forwarding probe; `/health/model-ready: 200`. The source checks these bindings in `services/core/src/config.rs` around lines 2231-2327. |
| Payments | Core payment config/token pair, private webhook secret and explicit RPC permissions; an independent Go host/payment credential; a reviewed external provider contract, durable proof/ledger acceptance, refund/recovery probe and financial reconciliation. The current Core config accepts `provider='stripe-test'` and `provider='stripe-live'` (`services/core/src/config.rs`); a test channel cannot pass this production gate. The preflight record uses `external` for a production provider, and accepting live configuration alone does not establish the required contract or reconciliation evidence. |
| Commerce | Core commerce config/token pair, explicit pay/settle/refund/read/delivery/reconcile permissions, Go store database and a Core funds adapter with independent credentials, plus delivery/recovery and financial reconciliation evidence. |
| Assistant | Core assistant config/token, an enabled model gateway, Web identity, durable assistant schema and an assistant credential distinct from identity, provider and financial tokens; Go must select `assistant` and supply its private cursor key. |
| Assistant tools | All assistant prerequisites plus durable agent/tool schema, business-facts read authority, browser confirmation and tool-market authority. |
| Tool market | Go `toolmarket` selection, dedicated storage, callback URL, socket/token, exactly 32-byte encryption key and independent host/witness/builtin credentials; Core Ready and credit-authority (`LegacyReady`) proofs. |

When the release requires the assistant agent, set `agent_required = true` in
the Extensions `[assistant]` section. The host then requires the selected
assistant module, independent assistant credential, Core socket and identity
credential, assistant database configuration, and cursor key. It refuses to
start if Core does not advertise both personal-text and agent capabilities;
it cannot silently fall back to a text-only assistant. The setting defaults
to `false` for existing configurations. This startup check does not verify the
agent's tool policies, financial behavior, or end-to-end readiness; those still
need the separate assistant acceptance evidence above.

Core enables the agent only when its version 1 assistant TOML contains an
`[agent]` table with `registry_file`, `registry_sha256` and nested
`[agent.policy]` / `[agent.policy.model]` tables. Personal text remains in
`[personal_text]`. The registry is a separate private reviewed JSON artifact;
its exact digest and the policy's model must
match the configured personal-text model policy. Enable Core assistant tools
and Web identity, then provision `enable_assistant_agent=true` and the required
tools/browser authority offline. Startup loads the registry and verifies the
installed agent journal before advertising agent capabilities. Missing or
invalid host bindings, schema or runtime authority fail startup. Fresh installs
contain the agent component; an existing installation requires a separate,
reviewed upgrade and cannot acquire it through startup. This wiring still
requires actual process, tool and financial acceptance for a release.

The source-level checks that enforce these boundaries are in
`services/core/src/config.rs` and
`services/extensions/internal/composition/{runtime.go,assistant.go,toolmarket.go}`.
No production credential value belongs in a preflight record or repository file.
An `extensions.host_credential` value of `absent` blocks the payments,
commerce, assistant, assistant tools and tool market gates. `reviewed` is an
input assertion, not proof that the credential exists or works.

## Assistant model protocol and platform funding

In **Plugin management**, open **Configure** on **AI assistant** to select the
host model. The picker lists models from the configured personal-text group
that the active routing release can serve and price under the host policy.
**Save** checks the displayed routing and pricing versions and asks you to
reload the configuration if either changed. Selecting the model from
`[personal_text]` restores the host default. Core uses the saved selection for
new personal-text grants and agent turns, while existing operations keep their
original model and price when their receipts are read.

Personal text supports a selected OpenAI Chat or Anthropic Messages route with
the `adapt` payload policy. Core selects the route, model, credential and frozen
price; the conversation cannot supply those values. The assistant retains its
ordered transcript and uses the existing relay converter for Anthropic request
and response formats. Billing reads the original supplier usage before response
conversion, including cache reads and cache creation. A missing counter is
treated as zero only when the frozen provider contract explicitly allows it;
other missing or inconsistent billable counters leave settlement pending.

Anthropic requests explicitly select `service_tier = "standard_only"` without
overriding the upstream workspace's inference geography. The assistant adapter
accepts an absent or null service tier, or `standard`; an absent or null
geography, or `global`; and absent or null server-tool usage, or known request
counters that are explicitly zero. Nonstandard tiers, non-global reported
geography, nonzero or unknown server-tool counters, and `iterations` remain
unsupported. This follows the provider's
[service-tier](https://platform.claude.com/docs/en/api/service-tiers) and
[geography](https://platform.claude.com/docs/en/manage-claude/data-residency)
contracts. The adapter validates a decoder-only copy and retains the original
response bytes and token receipt. A terminal Anthropic stream delta must include
its own cumulative `output_tokens`; the start-frame count cannot serve as the
final receipt. Existing counter-decrease checks still apply.

A successful model response by itself
does not authorize a gift or a wallet credit.

An operator can configure platform funding in the private version 1 assistant
TOML. The following IDs are illustrative; obtain the actual user and personal
account IDs from the committed identity mapping. They need not be equal.

```toml
[personal_text.service_funding]
payer_user_id = 90001
payer_account_id = 2
```

Omitting `service_funding` retains the existing funding behavior. Platform
funding and `sponsor` are mutually exclusive: the latter retains its new-user
eligibility, gift-tools requirement, call limit, budget and expiry rules. For
platform funding, each new grant and fresh reservation checks that the fixed
payer is an enabled superadmin with the configured active personal account.
The authenticated conversation owner remains the actor. Core reserves and
settles against the platform wallet using `wallet_only`; the grant budget,
account budgets, available balance and frozen per-request quote still apply.
An already dispatched charge can finish settlement or refund after the payer
is disabled, so that a later authority change does not strand its reservation.

Platform funding does not enable gift tools or change promotional eligibility.
Configure and review `gift_tools` and Core's gift policy separately. Anthropic
gift calls pass the same fixed-tool and typed-argument checks as OpenAI calls;
Core retains the original tool ID and response digest and rejects duplicate
argument fields, injected payer/owner IDs and parallel gift decisions.

Existing Core81 installations use the explicit offline owner procedure
`core-assistant-service-funding-v1` to reach Core82. Complete any importers bound
to the exact Core81 contract before this upgrade. The upgrade preserves
financial rows, grants, sequence state and predecessor permissions, and records
an immutable owner-only receipt. Normal service startup performs no DDL.

The implementation is in
[the funding policy](../services/core/src/assistant_model/service_funding.rs),
[assistant charging](../services/core/src/billing/charging/assistant.rs),
[protocol transport](../services/core/src/model_http/assistant_anthropic.rs) and
[the explicit upgrade](../services/core/src/core_schema/upgrade/assistant_service_funding.rs).
The [combined protocol and wallet test](../services/core/tests/assistant_model/service_funding/anthropic.rs)
uses a local synthetic upstream and a real isolated database. Test source is
not execution evidence; current results belong in [architecture progress](progress.md).

## Agent gift execution and configured search

The reviewed [version 2 tool registry](../contracts/assistant-tools-v2.json)
contains 23 tools, including `search_web`, the fixed Core
`prepare_new_user_gift` tool, and the existing Core `discover_tools` and
`end_conversation` control entries. Existing version 1 registry bytes remain unchanged.
Choose one reviewed registry and bind its exact hash in the private assistant
configuration. The registry is a supported-tool declaration; the host's
`allowed_tools` set and each domain's permissions still limit execution.

Agent gift issuance requires `gift_tools = true` in the model policy and the
fixed gift tool in `allowed_tools`. The policy must also match the configured
personal-text model policy, including its funding account. A browser cannot
choose the recipient, funding account or journal. Core rechecks the live Session,
frozen proposal and real settled model round, then applies the existing gift
eligibility and cap. The tool result, decision and canonical gift journal commit
in one transaction. Component83's deferred guards reject a successful tool result
without its gift, or an Agent gift without its completed tool result. Replaying
the same operation returns its original receipt; zero, denial and another
operation's existing gift explicitly report `issued = false`.

Personal-text gift execution retains its separate streaming authority. The Agent
entry does not broaden that entry to accept arbitrary settled requests. See
[the typed executor](../services/core/src/gift/agent.rs) and
[the schema binding](../services/core/schema/assistant_agent_gift.sql).

Exa is configured in Extensions `[assistant.search]` with `provider = "exa"`,
`endpoint`, `api_key_file` and `timeout_ms`. The API key is a private file;
the host requires Agent mode and caps the timeout at 30 seconds. When search is
not configured, the host does not advertise its executor. Each Exa request
explicitly selects `type = "auto"`, `numResults = 10` and highlights, without
requesting page summaries or additional results. The built-in tool price does
not establish that the external search provider is free. Verify the configured
account and [current provider pricing](https://exa.ai/pricing), and include the
external cost in the operator preview. Per-request limits do not establish an
aggregate monthly spending cap.

## Explicit Core81 to Core84 sequence

Complete imports at their frozen schema versions before advancing to the next
suffix. A newer binary accepts these explicit procedures because each one binds
its own predecessor and successor; ordinary startup still performs no DDL.

| Installed version | Explicit owner procedure | Successor |
| --- | --- | --- |
| Core81, after the reviewed81 imports | `core-assistant-service-funding-v1` | Core82 |
| Core82 | `core-assistant-agent-gift-v1` | Core83 |
| Core83 | `core-current-connection-history-v1` | Core84 |

The new current-history entry is independent of the historical
`core_history.connection_snapshot` contract. It requires the committed c343
current-identity receipt, the A513 current-key receipt and its separate outer
rights receipt, all bound to the same original source. It maps source keys to
their actual imported credential IDs. The observed maximum credential ID is
audit evidence; runtime classification never uses it to decide whether a key
was imported.

The offline owner inserts the complete credential map and source snapshot in
one SERIALIZABLE transaction. The source includes all retained user, token,
OAuth-grant and developer-request observations. The current-history entry does
not create credentials, move money, allocate sequence values or activate
authority. Existing source history, committed maps and upgrade receipts are
immutable. An already enabled community reader receives only the six snapshot
columns and target credential ID it consumes. Startup rejects missing required
columns or accidental access to private import columns and owner receipts.

[The source partition tool](../scripts/current_key_source_partition.py)
verifies the complete original export before deriving ordinary keys and the
separate internal-record archive. It does not convert internal billing handles
into ordinary API keys or dispose of active OAuth refresh families.
[The current-history tool](../scripts/current_connection_history.py) exposes
only `source-preview` on its CLI. It requires the exact reviewed support
bundles, including the original Rust identity files required by the frozen
validator:

```sh
python3 -B scripts/current_connection_history.py source-preview \
  --support-scripts /absolute/private/reviewed-a513/scripts \
  --support-scripts /absolute/private/reviewed-full81 \
  --support-scripts /absolute/private/reviewed-fa57/scripts \
  --evidence-file /absolute/private/current-history-source.json \
  --output /absolute/private/current-history-preview
```

This command writes a private derived preview and performs no source or target
queries. Its successful source validation is still pending actual committed
identity and key receipts. The reviewed offline owner controller invokes the
separate prepare/validate/execute interfaces with real target observations and
the required approval. Do not replace those receipts with a proposal, use an
old schema's importer on Core84, or copy a source-preview status into the
completed migration record below.

## Sanitized record and command

The record must use this top-level shape:

```json
{
  "protocol": "core-cutover-preflight.v1",
  "scope": "production-cutover",
  "host": "ArchDmit",
  "observed_at": "<ISO-8601 timestamp with timezone>",
  "source_commit": "<40 lowercase hex commit>",
  "source_revision": "<64 lowercase hex>",
  "check_db_evidence_sha256": "<64 lowercase hex>",
  "release": {
    "source_commit": "<same 40 lowercase hex commit>",
    "core_artifact_sha256": "<64 lowercase hex>",
    "extensions_artifact_sha256": "<64 lowercase hex>",
    "sbom_sha256": "<64 lowercase hex>",
    "schema_fingerprint_sha256": "<64 lowercase hex>",
    "provider_matrix_sha256": "<64 lowercase hex>",
    "identity_evidence_sha256": "<64 lowercase hex>",
    "financial_evidence_sha256": "<64 lowercase hex>",
    "edge_config_sha256": "<64 lowercase hex>",
    "runtime_release_identity_sha256": "<64 lowercase hex>",
    "runtime_source_commit": "<same 40 lowercase hex commit>",
    "runtime_core_artifact_sha256": "<same core artifact digest>",
    "runtime_extensions_artifact_sha256": "<same extensions artifact digest>",
    "previous_writer_epoch": 7,
    "writer_epoch": 8
  },
  "core": {
    "ready": {
      "http_status": 200,
      "database": true,
      "model_ready_status": 503,
      "capabilities": {
        "models": false,
        "models_streaming": false,
        "payments": false,
        "commerce": false,
        "assistant": false,
        "assistant_tools": false,
        "tool_market": false
      }
    },
    "public_edge": {"mode": "legacy", "candidate_selected": false},
    "contracts": {
      "model_forwarding": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "payments": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "commerce": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "assistant": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "assistant_tools": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "tool_market": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"}
    }
  },
  "extensions": {"ready_http_status": 200, "modules": [], "host_credential": "absent"},
  "identity": {
    "migration_status": "identity_only",
    "mapping_status": "passed",
    "target_comparison_status": "passed",
    "legacy_inventory": {
      "sessions": 723,
      "api_keys": 815,
      "passkeys": 8,
      "oauth_bindings": 235,
      "oauth_server_tokens": 1214,
      "mfa_factors": 2,
      "mfa_backup_codes": 8
    },
    "source_archive_sha256": "<64 lowercase hex>",
    "source_export_sha256": "<64 lowercase hex>",
    "source_baseline_proof_sha256": "<64 lowercase hex>",
    "mapping_sha256": "<64 lowercase hex>",
    "mapping_file_sha256": "<64 lowercase hex>",
    "source_target_id_map_sha256": "<64 lowercase hex>",
    "identity_contract_sha256": "<64 lowercase hex>",
    "target_import_sql_sha256": "<64 lowercase hex>",
    "target_receipt_sha256": "<64 lowercase hex>",
    "reauth_required": 2172,
    "rekey_required": 815,
    "reenroll_required": 18,
    "actions": {
      "sessions": "reauth_required",
      "api_keys": "rekey_required",
      "passkeys": "reenroll_required",
      "oauth_bindings": "reauth_required",
      "oauth_server_tokens": "reauth_required",
      "mfa_factors": "reenroll_required",
      "mfa_backup_codes": "reenroll_required"
    }
  },
  "migration": {
    "status":"blocked",
    "unknown_liabilities":82,
    "reconciliation":"not_run",
    "writer_fenced":false,
    "active_writers":1,
    "legacy_workers_stopped":false,
    "financial_evidence_sha256":"<64 lowercase hex>",
    "pending_preview_evidence_sha256":"<64 lowercase hex>",
    "provider_contract_evidence_sha256":"<64 lowercase hex>",
    "provider_reconciliation_evidence_sha256":"<64 lowercase hex>",
    "provider_io_performed":false,
    "target_writes":0,
    "ledger_writes":0,
    "spendable_writes":0,
    "receipt_writes":0
  }
}
```

The `identity` block is strict. `legacy_inventory` must contain exactly the
seven non-negative counts shown above. `mapping_status` and
`target_comparison_status` must both be `passed`; `migration_status` must be
`identity_only` or `completed`. Each inventory class has a fixed safe action:
sessions and OAuth records require reauthentication, API keys require new-key
creation, and passkeys/MFA records require re-enrollment. The action counts may
be `true` (all rows) or a non-negative integer covering the rows assigned to
that action. Every hash field is required and must be bound to the same frozen
source export, mapping, identity contract, target import SQL and target receipt;
the preflight checks format and presence, while the independent runbook checks
the bytes. Any uncovered, differently classified or unbound legacy row blocks
the identity gate and therefore the complete cutover result.

## Build the identity block from an apply directory

`scripts/build-identity-cutover-evidence.py` reads a private identity apply
directory and emits only the sanitized `identity` block. It verifies the
identity manifest/report, mapping and source proof files, generated SQL,
target-import result, target receipt, and an independent identity PASS result.
It checks their SHA-256 bindings and target instance boundary without printing
any file contents. The normal synthetic contract uses
`independent-pass.private.json`; the existing rehearsal layout is also
recognized when the input is `.../identity-apply`, with the target receipt in
its parent and `independent-identity/identity-independent-result.private.json`
beside it.

The existing rehearsal PASS file does not contain the seven legacy credential
counts or reauthentication plan. The builder therefore returns `not_run` until
the same directory (or its parent) contains a sanitized
`identity-inventory.private.json`; it never infers those counts from the source
dump or empty Core credential tables.

```sh
python3 -B scripts/build-identity-cutover-evidence.py \
  --input /absolute/private/identity-apply \
  --output /absolute/private/identity-evidence.json
```

The command exits `0` only for `PASS`; incomplete evidence is a sanitized
`not_run` or `blocked` result and exits `1`.

Run it from a reviewed evidence directory:

```sh
python3 -B scripts/core-cutover-preflight.py \
  --evidence /absolute/private/core-cutover-preflight.json \
  --expected-source-commit <40 lowercase hex candidate commit> \
  --output /absolute/private/core-cutover-preflight-result.json
```

The output includes each gate's `missing` list, `unsafe_enablement` for a
capability that is claimed enabled without a complete contract, and the bound
source revision. The output's `secret_values_checked="schema_only"` means that
schema validation prevented secret-shaped fields; it is not provider or
credential verification. Independent host, database, provider and browser
acceptance remain required.

When `--expected-source-commit` is supplied, the candidate commit is an
external deployment input and must match the record and runtime source commit.
The release financial evidence digest must also equal the canonical migration
financial evidence digest; a digest-shaped value from another evidence bundle
is blocked.

The migration block also requires four strict evidence SHA-256 bindings: the
financial review, pending-liability preview, provider contract, and provider
reconciliation evidence. It rejects any reported provider I/O or non-zero
target, ledger, spendable, or receipt write counter. A hand-written
`reconciliation="passed"` with suitably shaped hashes and zero-write
assertions can pass this consistency check without the evidence files being
examined. The current 82-row preview has no provider reconciliation evidence
and remains blocked.

Local synthetic checks (no host access) are:

```sh
python3 -B scripts/test_core_cutover_preflight.py
```
