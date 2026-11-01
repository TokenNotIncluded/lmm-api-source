---
type: operations
title: Configuration, startup, schema, and release lifecycle
description: Shows how to select development tools, configure Core and Extensions independently, separate offline owner actions from runtime, and distinguish builds from deployment.
tags: [operations, configuration, startup, schema, release]
sources:
  - id: openwiki-source-70991026f1538e8215a8789e
    resource: repo://docs/development.md
  - id: openwiki-source-b50f9dbe9857371baf06928b
    resource: repo://docs/release-architecture.md
  - id: openwiki-source-c8f46b83e813ffc1055b9d7c
    resource: repo://services/extensions/internal/app/configuration.go
generated: { by: "codex", at: "2026-11-01T04:36:16.003Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Configuration, startup, schema, and release lifecycle

## Prepare only the relevant toolchain

The root `mise.toml` is the version authority. Prepare tools for the component being changed: Rust for Core, Go for Extensions, Bun/Node for Web, and the pinned CLI submodule only for CLI work. The project guide explicitly says Web setup is not required for Rust or Go development, and database tests use isolated PostgreSQL. See [development](../../docs/development.md).

## Core configuration and database roles

Core accepts an explicit absolute TOML path with `lmm-core --config`. Database credentials are referenced through a private file. Keep the offline installation/owner identity separate from the restricted network runtime. `init-db` is an explicit first-install command for a new empty database; normal startup verifies installed schema and runtime authority but does not create roles, install schema, or upgrade existing data. `check-db` is a read-only check of the selected configuration and installed database contract.

First-owner creation and schema upgrades are also explicit offline commands. Choose an upgrade procedure that matches the exact installed contract; do not infer it from the current source version. See [Core schema lifecycle](../architecture/core-identity-and-data.md).

## Extensions configuration

The Go host accepts its own TOML configuration with an absolute host token path. Configure selected modules explicitly. Modules that use Core require both a Unix socket and the corresponding credential file; storage modules require their own database name and credential file. The parser rejects unknown TOML fields, invalid paths, and incomplete path/credential pairs. Legacy environment compatibility exists only for explicit test compatibility and should not be used as a new deployment template.

Run the Core and Extensions as separate processes. Extensions do not inherit Core's database URL. Web static assets are built and served separately. See [service boundaries](../architecture/system-boundaries.md) and [Extensions host](../architecture/extensions-and-integrations.md).

## Build, package, and deploy are separate

Build only the required component from a fixed source revision. Distribution archives carry component, version, revision, platform, and file digests; verification checks an archive against the trusted source identity. A package or preview workflow is not itself a formal release, installation, service restart, or production acceptance.

Schema installation and upgrades are not package hooks. Runtime startup does not automatically initialize or migrate a database. Confirm the target installation procedure and database state separately, then read back the running component identity and exercise the affected business path. A health response or successful static build does not prove model or financial workflows. See [release architecture](../../docs/release-architecture.md).
