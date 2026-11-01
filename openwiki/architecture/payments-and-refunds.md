---
type: architecture
title: Payments, provider evidence, and refunds
description: Traces payment preparation, provider proof, Core ledger transitions, and evidence-bound refund handling across Rust Core and Go Extensions.
tags: [payments, refunds, ledger, provider-evidence, extensions]
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
sources:
  - id: openwiki-source-08f62454138cd168ac34dfdb
    resource: repo://services/core/src/payments/mod.rs
  - id: openwiki-source-6f2d1fb9912825fa21ad3d68
    resource: repo://services/core/src/payments/rpc.rs
  - id: openwiki-source-405a849f981034033f55c31d
    resource: repo://services/core/tests/payments_process.rs
  - id: openwiki-source-65292e405c537c6c5b6d494b
    resource: repo://services/core/tests/payments_refund_process.rs
generated: { by: "codex", at: "2026-11-01T04:43:29.750Z" }
---

# Payments, provider evidence, and refunds

## Authority split

Rust Core owns payment intents, verified provider transitions, and money movement through its existing controlled ledger. Provider network calls do not run inside the Core metadata transaction. Go Extensions compose provider adapters and persist provider-specific state, then call Core over a separately permissioned payment RPC surface. See [Core identity and persistence](core-identity-and-data.md) and [Extensions](extensions-and-integrations.md).

## Intent and proof flow

The extension prepares a checkout against an enabled configured adapter and an exact account, key, amount, and currency. Core validates the operation against configured provider/channel identity and the authenticated intent; browser-supplied or callback values do not define the provider binding. Signed or independently verified provider evidence is ingested and reconciled against the persisted intent before Core issues a ledger credit receipt. Reconciliation uses stable operation keys to recover uncertain outcomes without applying duplicate credits.

## Refund state

Refunds have separate user authorization, hold, provider dispatch, evidence, and commit/release operations. The RPC advertises only operations enabled by explicit permissions. A provider dispatch with a lost or uncertain response is treated as unknown until authoritative provider evidence resolves it; an unverified callback cannot itself prove a refund or reverse a credit. Durable refund state and ledger effects are reconciled using the same operation identity.

## Verification scope

Core's `payments_process.rs` exercises a synthetic signed payment proof across Core and Go process boundaries. `payments_refund_process.rs` exercises provider response loss, restart, and later evidence resolution using synthetic fixtures. These process tests do not perform live payment-provider calls or validate production merchant configuration. See [verification map](../testing/verification-map.md).
