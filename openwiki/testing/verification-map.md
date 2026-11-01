---
type: testing
title: Test layers and verification map
description: Explains what unit, database, process, browser, and independent acceptance checks cover, how to choose the narrowest useful check, and why a passing layer is not full product acceptance.
tags: [testing, verification, acceptance, postgres, browser]
sources:
  - id: openwiki-source-20e5f0d1ece48339c9d51ab0
    resource: repo://docs/agent-workflows.md
  - id: openwiki-source-3dd29efd00addae4a074df2d
    resource: repo://docs/quality.md
  - id: openwiki-source-c6eeace527e5c716fc381ead
    resource: repo://services/core/tests/model_http.rs
  - id: openwiki-source-ae6472941387fd5b170e4349
    resource: repo://services/extensions/internal/composition/integration_test.go
generated: { by: "codex", at: "2026-11-01T04:36:16.003Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Test layers and verification map

## Choose checks by changed behavior

The repository's working guide maps changes to their focused checks: Rust Core tests, Go package tests, Web tests/typecheck, CLI submodule tests, and distribution tooling each have separate commands. Database or financial changes require isolated real database checks for the affected behavior. Do not run every component suite for a local change, and do not substitute fixed-success mocks for the internal identity, permission, routing, or ledger path.

`just check` aggregates several source and component checks but excludes the CLI and all independent database modules. The quality tool's `quick`, `related`, and `full` modes overlap; choose the one matching the acceptance goal. A broad aggregate may still leave independent business and resource acceptance unrun.

## What test layers establish

- **Unit and package tests** check local parsing, state transitions, adapters, and component behavior. Examples include the Go composition tests, Web node tests, and provider package tests.
- **Database tests** exercise schema, grants, persistence, concurrency, idempotency, and resulting data using isolated PostgreSQL. A passing synthetic fixture does not establish production data migration or external provider behavior.
- **Process and protocol tests** start real components and verify their RPC/HTTP interaction. Check whether the authority or supplier is a real service or a controlled fixture; the Go composition integration test explicitly uses a synthetic Core authority.
- **Browser acceptance** checks rendered pages, navigation, error states, role-specific behavior, and real service integration in an isolated environment. A successful build or HTTP 200 is insufficient.
- **Independent business acceptance** freezes the tested source/artifact and independently checks the requested business outcome. Developer self-tests, test counts, and a package or deployment alone do not constitute independent sign-off.

## Example: model streaming

`services/core/tests/model_http.rs::streaming_chat_and_responses_commit_real_evidence_then_settle_once` uses an isolated PostgreSQL fixture and loopback provider. It verifies Chat and Responses streaming, persisted ordered checkpoints, terminal usage, exactly-once settlement, duplicate rejection, and upstream body cleanup. This is strong evidence for that Core path with the controlled provider; it does not prove external-provider compatibility or deployed service acceptance. See [model routing](../architecture/model-routing-and-billing.md).

## Reporting

Keep each state distinct: local self-test, independent acceptance, source push, package build/sign, release, and deployment. Record the command, exact input revision, exit status, and limits. Report skipped, blocked, failed, and not-run checks as such. The project guide requires stopping on repeated same-cause failure rather than switching to a weaker mock to obtain a pass.

See [development workflow](../../docs/agent-workflows.md) and [quality acceptance guide](../../docs/quality.md) for current command selection and full acceptance procedure.
