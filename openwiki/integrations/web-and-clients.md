---
type: integration
title: Web, CLI, and provider integrations
description: Maps the static React console, separate Rust CLI, and editor/provider packages to their service boundaries, development commands, and representative tests.
tags: [integration, web, cli, providers, testing]
sources:
  - id: openwiki-source-ed77bc20b48d193bace3dde6
    resource: repo://.gitmodules
  - id: openwiki-source-99de51df25f29bfc72caf823
    resource: repo://apps/web/package.json
  - id: openwiki-source-70991026f1538e8215a8789e
    resource: repo://docs/development.md
generated: { by: "codex", at: "2026-11-01T04:43:29.750Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Web, CLI, and provider integrations

## Static Web console

`apps/web` is a private React workspace package (`@lmm/web`). Its development server and build are managed through the root Bun workspace. A production build emits static assets; the package does not become another API service. Web uses configured service endpoints and relies on Core/Extensions for authorization and business operations (see [system boundaries](../architecture/system-boundaries.md)).

The Web package's test command runs a focused Node callback test followed by the serial test runner. Its scripts also expose separate typecheck, lint, formatting, bundle-size, and build checks. Choose the command that covers the change; a successful static build does not establish backend business behavior.

## CLI

`apps/cli` is a separate Rust client repository included as a Git submodule. Its executable is `lmm`; the CLI connects to server APIs and performs user-side operations but does not decide final authorization or charging. Its repository identity is verified by the parent Git submodule entry and development guide. The submodule is not initialized in this source checkout, so its manifest and package tests are outside the evidence available here.

For CLI work, initialize the pinned submodule as described in [development](../../docs/development.md), make and verify changes in that repository, and update the pointer only after the source change is committed and pushed. This main source repository's CLI path is a dependency boundary, not a second copy of the CLI implementation.

## Editor and provider packages

`packages/` includes separate Git submodules for Pi, Zed, VS Code, OpenCode, DSH, and CodeWhale integrations. Their pinned repository identities are visible in `.gitmodules`; the submodule worktrees are not initialized here, so their current manifests, implementation details, and tests are not verified by this page.

When changing an integration, inspect the corresponding pinned repository, use its own test/build instructions, and verify the matching protocol contract. A repository-local test does not establish that a live upstream host, account, or deployed service is available.

## Cross-component path

Web and client/provider code submit requests to the service APIs. The Core decides identity, permissions, model route, and funds. Selected Go Extensions expose extension business operations and compatibility routes and call Core through its private versioned RPC. See [Extensions integrations](../architecture/extensions-and-integrations.md) and [test layers](../testing/verification-map.md).
