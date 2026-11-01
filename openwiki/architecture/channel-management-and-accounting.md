---
type: architecture
title: Channel management and accounting
description: Documents Core managed channel publication, encrypted provider credentials, and display-only accounting continuity.
tags: [channels, routing, accounting, credentials, audit]
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
sources:
  - id: openwiki-source-773940de3a6d666e2f498423
    resource: repo://services/core/schema/channel_accounting.sql
  - id: openwiki-source-03113b5eda6afa983b51264c
    resource: repo://services/core/src/channel_http/control.rs
  - id: openwiki-source-e60c87b2b125c19c754f90f1
    resource: repo://services/core/src/channel_http/control/accounting.rs
  - id: openwiki-source-fd82bc5d4bb7a66447a6a2f6
    resource: repo://services/core/src/channel_http/control/vault.rs
  - id: openwiki-source-1718e37632ccd6c334358b19
    resource: repo://services/core/tests/channel_accounting_schema.rs
  - id: openwiki-source-13baabec5d2d92bfa8a75969
    resource: repo://services/core/tests/channel_control.rs
generated: { by: "codex", at: "2026-11-01T04:43:29.750Z" }
---

# Channel management and accounting

## Publication authority

Core's `ManagedChannels` owns runtime channel control. At startup, `recover` reads an existing durable publication and installs it; it does not initialize the database or create a new publication. Runtime publication validates provider bindings and credential references, compares the caller's expected routing/pricing version with the durable state, and records channel accounting continuity in the same transaction. Core writes the durable routing publication and an audit record, commits, and only then installs the new serving snapshot. A failed audit or commit therefore does not publish a new in-memory snapshot. See [model routing and billing](model-routing-and-billing.md).

## Provider credentials

Channel secrets are held by the Core gateway and persisted as encrypted credential material. The vault uses AES-256-GCM with a derived key, a fresh nonce, and associated data binding ciphertext to the immutable credential ID and revision. Empty, oversized, or non-printable secrets are rejected. Runtime edits must keep credentials and provider contexts bound to their intended upstream; enabling a channel requires an actual loaded key and provider contract.

## Accounting continuity

The `core_channels` schema records channel identity, historical counter baselines, and provider balance observations. These are continuity and display facts; they are not a second financial ledger. Native usage and refunds remain grounded in the charging records. New identities receive an unknown balance observation until a provider observation exists. Copy operations either carry forward the source's settled/refunded counters and coverage or explicitly reset counters; retired IDs remain reserved. The schema and publication path keep these changes transactional with routing releases.

## Tests and scope

`channel_control.rs` exercises version conflicts, audited publication, failure behavior, provider credential requirements, and restricted runtime database access. `channel_accounting_schema.rs` checks schema constraints and privilege boundaries. These are Core service and PostgreSQL integration tests; they do not prove production provider credentials or external provider behavior.
