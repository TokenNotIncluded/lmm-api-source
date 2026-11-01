---
type: architecture
title: Extensions host and service integrations
description: Describes how the Go Extensions process composes selected business modules, calls the Rust Core over private Unix-socket RPC, and owns separate extension persistence.
tags: [architecture, go, grpc, integrations, extensions]
sources:
  - id: openwiki-source-e7b99199bcb25bf58a5e1ee3
    resource: repo://services/extensions/internal/composition/runtime.go
  - id: openwiki-source-dc49d35c62b7895037fdc309
    resource: repo://services/extensions/internal/coreclient/client.go
generated: { by: "codex", at: "2026-11-01T04:36:16.003Z" }
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
---

# Extensions host and service integrations

## Host composition

`lmm-extensions` enters through `services/extensions/cmd/extensions`; its normal path calls the app runner, which parses configuration and builds a selected module set. The composition package validates module dependencies before constructing clients and storage. An empty selection can run without connecting to Core; selected modules require Core RPC configuration. Some modules have additional prerequisites: support requires store, and commerce credentials require store selection.

The host owns lifecycle coordination, module admission, HTTP compatibility handlers, and the assembly of payment, store, support, assistant, operations, and tool-market services. This is an extension host rather than a second authority for Core identity or model traffic. See [system boundaries](system-boundaries.md).

## Core RPC boundary

`internal/coreclient` is the Go modules' only Core access path. It has no Core database connection, authorization cache, or model relay. The client connects through an absolute Unix socket, reads bounded private credentials from restrictive regular files, and applies bounded message sizes, request timeouts, and an in-flight limit. The call context carries the service credential and protocol marker; user credentials are supplied only to methods that require user authorization.

This interface preserves the Core as the owner of account authorization and financial decisions. Go modules may request operations through typed/versioned RPCs but cannot query or write Core tables. Keep RPC contract changes coordinated with the Core implementation and contract fixtures.

## Independent persistence and recovery

Extension modules own independent storage selected by their configuration. Composition assembles storage bundles and validates the grants and forbidden operations each module requires; payment isolation is also represented explicitly. Core credentials and extension host credentials must be independent.

Background recovery starts explicitly after listener readiness. A bounded polling loop runs recovery work for configured payments, store deliveries, and tool-market settlements. Each task must acquire host admission and receives a timeout; saturation defers work to a later tick rather than building an unbounded queue. Shutdown cancels and joins the worker before closing resources.

## HTTP compatibility and tests

The `webcompat` packages adapt supported legacy/public Web routes to the new module services. Treat route compatibility as an adapter contract: a handler being present does not prove every historic behavior is implemented. Focused tests live alongside composition and webcompat packages; database integration tests require isolated PostgreSQL fixtures. See [Web and client integrations](../integrations/web-and-clients.md) and [verification map](../testing/verification-map.md).
