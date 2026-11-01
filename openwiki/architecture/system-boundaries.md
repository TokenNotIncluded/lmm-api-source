---
type: architecture
title: System boundaries and request flow
description: Maps the three executable programs and static Web app, their authoritative responsibilities, and the contracts callers use to cross service boundaries.
tags: [architecture, boundaries, rust, go, web, contracts]
sources:
  - id: openwiki-source-e33cd5bff4e08eda846efc37
    resource: repo://contracts/proto/INTEGRATION.md
  - id: openwiki-source-74aac440cae42cda50273199
    resource: repo://docs/core-migration.md
  - id: openwiki-source-e2e3e780f2aa07676cbf31b8
    resource: repo://services/core/src/main.rs
  - id: openwiki-source-dc49d35c62b7895037fdc309
    resource: repo://services/extensions/internal/coreclient/client.go
generated: { by: "codex", at: "2026-11-01T04:36:16.003Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# System boundaries and request flow

## Runtime components

The repository describes three independent executables plus a separately built static Web app:

- **`lmm-core` (Rust)** owns identity, permissions, accounts, funding and ledger state, budgets, subscriptions, model routing, and provider forwarding.
- **`lmm-extensions` (Go)** hosts selected extension business modules such as store, tools, assistant, operations, and payment adapters. It does not own Core identity, directly change Core balances, or connect to the Core database.
- **`lmm` (CLI)** is a separate client that connects to the services and performs user operations; final authorization and charging remain server responsibilities.
- **`apps/web`** is a React application built into static files and served separately.

This separation lets Core operate without the Go process. A module's source or schema can exist without being enabled in the running host; composition and configuration decide which interfaces are active.

## Request paths

A model API request is handled by Core: Core authenticates the caller, selects host-configured routing and price, reserves/settles through its billing and ledger components, and contacts the provider. The client cannot supply the authoritative URL, upstream secret, payer, or price. See [model routing and billing](model-routing-and-billing.md).

Extension requests reach Go's selected host modules. When a module needs Core identity or financial authority, it calls the versioned Core RPC over a private Unix socket. Go modules persist their own extension state in separately configured databases; they do not query Core tables. The Go host can expose selected compatibility routes for the Web application. See [Extensions host](extensions-and-integrations.md) and [Web and clients](../integrations/web-and-clients.md).

The Web frontend is a caller and static asset, not an authority boundary. It talks to the configured service APIs; authorization, account state, and charges are decided server-side.

## Contracts and operations

`contracts/` holds cross-service and public protocol definitions. Protobuf service definitions and generated bindings are not sufficient evidence that a business operation is implemented or mounted: check the server registration, host composition, and behavior tests together. Similarly, a route adapter or component schema does not prove historical parity or a production cutover.

Fresh database installation, first-owner creation, schema upgrade, and read-only database checking are explicit `lmm-core` CLI operations. They do not add a separate administrative daemon. Normal service startup verifies its configured runtime authority and schema, then starts the process's own interfaces.

## Deployment status

Architecture documentation describes ownership and expected interactions; it does not assert that a particular candidate is deployed or has passed full independent business acceptance. Report source, tests, packages, release, and deployment as separate states. The current development guide and acceptance records are the source for those states.
