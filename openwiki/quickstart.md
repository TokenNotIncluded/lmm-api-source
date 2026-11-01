---
type: guide
title: Quickstart and topic map
description: Entry point for contributors to locate the Core, Extensions, Web, CLI, operations, and testing guidance without treating source presence as deployment acceptance.
tags: [quickstart, navigation, development, architecture]
sources:
  - id: openwiki-source-20e5f0d1ece48339c9d51ab0
    resource: repo://docs/agent-workflows.md
  - id: openwiki-source-74aac440cae42cda50273199
    resource: repo://docs/core-migration.md
  - id: openwiki-source-70991026f1538e8215a8789e
    resource: repo://docs/development.md
  - id: openwiki-source-b50f9dbe9857371baf06928b
    resource: repo://docs/release-architecture.md
generated: { by: "codex", at: "2026-11-01T04:43:29.750Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Quickstart and topic map

## Start with the component

This checkout is a multi-component source repository. Its documented runtime consists of Rust Core, Go Extensions, a separate CLI submodule, and a React static Web app. Read [system boundaries](architecture/system-boundaries.md) before changing a cross-service contract.

| Work area | Start here | Main source and tests |
| --- | --- | --- |
| Identity, permissions, schema, and authoritative funds | [Core identity and data](architecture/core-identity-and-data.md) | `services/core/src/identity`, `billing`, `core_schema.rs`; `services/core/tests/` |
| Model API, routing, provider transport, and billing | [Model routing and billing](architecture/model-routing-and-billing.md) | `services/core/src/model_http`, `relay`, `routing`; `services/core/tests/model_http.rs`, `tests/model_process/` |
| Managed channels, provider secrets, and accounting continuity | [Channel management and accounting](architecture/channel-management-and-accounting.md) | `services/core/src/channel_http/control`, `services/core/schema/channel_accounting.sql`; `services/core/tests/channel_control.rs`, `channel_accounting_schema.rs` |
| Payment intents, provider proof, and refunds | [Payments and refunds](architecture/payments-and-refunds.md) | `services/core/src/payments`, `services/extensions/internal/modules/payments`; Core payment process tests |
| Store, payments, assistants, tools, and compatibility APIs | [Extensions host](architecture/extensions-and-integrations.md) | `services/extensions/internal/composition`, `modules`, `webcompat`; colocated Go tests and integration fixtures |
| Console, CLI, editor/provider packages | [Web and clients](integrations/web-and-clients.md) | `apps/web`, `apps/cli` submodule, `packages/` |
| Tool setup, configuration, fresh install, release | [Configuration and lifecycle](operations/configuration-and-lifecycle.md) | `docs/development.md`, `docs/filesystem-layout.md`, `docs/release-architecture.md` |
| Selecting checks and interpreting results | [Verification map](testing/verification-map.md) | `docs/agent-workflows.md`, `docs/quality.md`, component test trees |

## Development flow

Read the root [`AGENTS.md`](../AGENTS.md), then the relevant component guide and matching tests. Prepare only the toolchain needed for that component. Keep test databases and credentials isolated. Run the narrow check for the changed behavior and record its exact result; broaden the check only when the change or requested acceptance requires it.

Core schema installation and upgrades are explicit offline operations. Normal startup is a separate path that checks the installed contract and runtime authority. Do not infer that optional routes are active just because their code or schema exists.

## Acceptance status

This Wiki describes the checked-out source and documented intended boundaries. It does not claim that every current uncommitted source change passed, that external-provider or browser acceptance is complete, or that a particular build is released or deployed. Report local self-test, independent acceptance, packaging, release, and deployment separately. See the [verification map](testing/verification-map.md).
