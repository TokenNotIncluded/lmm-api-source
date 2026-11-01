---
type: architecture
title: Core identity, persistence, and schema lifecycle
description: Explains the Rust Core's ownership of identity and authoritative financial state, and how PostgreSQL installation, runtime authority checks, and schema upgrades are separated.
tags: [architecture, rust, identity, postgres, schema, security]
sources:
  - id: openwiki-source-74aac440cae42cda50273199
    resource: repo://docs/core-migration.md
  - id: openwiki-source-4d9265116dd8ec958fae27e3
    resource: repo://services/core/schema/README.md
  - id: openwiki-source-d342b5ee31f6780437a038e1
    resource: repo://services/core/src/core_schema.rs
  - id: openwiki-source-e2e3e780f2aa07676cbf31b8
    resource: repo://services/core/src/main.rs
  - id: openwiki-source-c58813d2d41f5a41efdfaa14
    resource: repo://services/core/src/runtime_database.rs
generated: { by: "codex", at: "2026-11-01T04:36:16.003Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Core identity, persistence, and schema lifecycle

## Ownership

The Rust `lmm-core` process is authoritative for identity, permissions, accounts, funding rules, ledger state, budgets, subscriptions, and model routing. Go Extensions use versioned Core RPCs and their own database; they do not receive Core database credentials. The Core's HTTP/runtime code creates its identity store from the Core pool, and configured model service is attached to that store (see [system boundaries](system-boundaries.md) and [model routing and billing](model-routing-and-billing.md)).

This boundary makes the Core database the authority for decisions involving who may act and how funds move. Extensions can own extension records but must call the Core for identity or financial authority.

## Configuration and runtime startup

`lmm-core` accepts an explicit absolute TOML configuration path; `serve` is the default command. Administrative commands are separate CLI operations. The serving path derives required database capabilities from enabled configuration, checks the effective runtime role, verifies the installed Core schema, and only then assembles stores and listeners. A configured model gateway also performs recovery of expired reservations and completed streams before readiness/listener binding.

Runtime database verification uses a read-only, repeatable-read transaction with a short statement timeout. It inspects effective privileges, including inherited and role-switch paths, and distinguishes unsafe privileges, missing privileges, and an unavailable check. This is an authority check, not a migration step. See [configuration and lifecycle](../operations/configuration-and-lifecycle.md).

## Installation and schema contract

Fresh installation is performed only by the explicit offline `init-db` operation. The compiled component list in `core_schema.rs` is fixed by the source build and includes disabled optional components as part of the whole Core schema contract. Installation rejects an occupied application database and applies the schema and component metadata transactionally; ordinary startup does not create or repair tables.

The contract has component fingerprints and a whole-catalog fingerprint. Startup compares the installed contract with the compiled definitions and live PostgreSQL catalog. This catches schema, function, and trigger drift while permitting runtime-role provisioning that does not alter those definitions. Module schema presence alone does not mean its HTTP surface is enabled: configuration and router composition remain separate.

## Upgrade boundary

Existing databases are not implicitly upgraded by startup, packaging, or fresh-install procedures. The binary exposes explicit offline upgrade commands with exact predecessor/target contracts and installer-owner configuration. Select and review the matching upgrade path for the actual database revision; a fresh empty-database install is not a general migration mechanism. The repository's PostgreSQL schema guide documents these constraints and distinguishes historical data import from schema installation.

## Focused evidence

- `services/core/src/core_schema.rs` defines the fixed compiled components, install contract, and catalog verification responsibilities.
- `services/core/src/runtime_database.rs` checks the network process's effective database authority in a read-only transaction.
- `services/core/tests/runtime_database.rs` exercises restricted-role behavior and capability-specific missing grants against PostgreSQL fixtures.
- `services/core/schema/README.md` describes the operational separation between owner provisioning, runtime startup, and explicit upgrades.
