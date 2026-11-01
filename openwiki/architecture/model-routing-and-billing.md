---
type: architecture
title: Model routing, provider calls, and billing
description: Traces model requests through host-owned authentication and routing, durable charge reservations, upstream execution, stream checkpoints, settlement, and recovery.
tags: [architecture, models, routing, billing, streaming, postgres]
sources:
  - id: openwiki-source-4b3014a59ba88d1fef43e7ee
    resource: repo://services/core/src/model_http.rs
  - id: openwiki-source-bd98f177aa92f6b9d0121694
    resource: repo://services/core/src/model_http/billing.rs
  - id: openwiki-source-c6eeace527e5c716fc381ead
    resource: repo://services/core/tests/model_http.rs
generated: { by: "codex", at: "2026-11-01T04:36:16.003Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Model routing, provider calls, and billing

## Host-owned routing and trust inputs

The model HTTP gateway runs in Rust Core. The host supplies routing snapshots, provider credentials, key-to-group grants, and charging policy; client request fields cannot choose the upstream URL, provider key, payer, or price. The gateway's charging and ledger adapters share the Core PostgreSQL pool. This keeps caller-controlled request data separate from the authority used to select a provider and calculate a charge.

Before dispatch, the gateway authenticates the credential, selects a configured route, freezes the selected price against the ledger's charge, and prepares a protocol-specific request. The current gateway maps supported OpenAI Chat, OpenAI Responses, Anthropic, and Gemini attempts to relay protocols; unsupported attempts fail as route unavailable. See [Core ownership](core-identity-and-data.md).

## Reservation, dispatch, and finalization

Billing reserves funds before upstream dispatch. The reservation records the selected request and charge context so provider output can be finalized against the same payer and pricing decision. Retry logic is deliberately narrow: only an explicitly rolled-back `Retry` result is replayed with the same operation identity. An unknown commit outcome is not a safe signal to dispatch again.

On a terminal provider result, the gateway reconciles reported usage and settles the charge through the Core billing/ledger path. If evidence is incomplete or the result is indeterminate, the hold remains pending authoritative evidence; recovery does not turn a timeout into an automatic release. Finalization retries are bounded by a deadline and limited to retryable transactional outcomes.

## Streaming and restart recovery

Text streaming persists ordered output checkpoints, including terminal provider usage, before the client response can be considered complete. This lets a trusted host reconcile a stream that completed immediately before process failure. Partial streams remain frozen; host recovery first handles expired reservations and then reconciles completed streams using the original charging operation. These recovery methods are host-only rather than HTTP, client, or Go operations.

Image work has a related durable receipt/recovery path, but it is implemented separately from text stream checkpoints. Model availability is opt-in configuration and readiness is separate from basic Core readiness.

## Tests and limits

`services/core/tests/model_http.rs` runs the gateway against isolated PostgreSQL and a controlled loopback provider. The streaming test checks Chat and Responses output, persisted ordered checkpoints, one settlement, duplicate rejection, and released upstream resources. Additional cases cover unknown holds and provider failures. These tests establish the behavior of the fixture and Core transaction path; they do not establish compatibility with every external provider or production credentials.

See [verification map](../testing/verification-map.md) for the boundary between focused tests and broader business acceptance.
