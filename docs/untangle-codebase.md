# Codebase untangling — 2026-11-06

This refactor implements twelve structural decisions across Core, Extensions, Web and offline import tooling. It preserves the tested external behavior and records the remaining verification limits.

Baseline: [90e2ce18896e90b23b5297513d1bd7d572abf6fb](https://github.com/TokenNotIncluded/lmm-api-source/commit/90e2ce18896e90b23b5297513d1bd7d572abf6fb).
Code result: [7a63d1cfbc71ea14ed8446cba4bd45752c996ffd](https://github.com/TokenNotIncluded/lmm-api-source/commit/7a63d1cfbc71ea14ed8446cba4bd45752c996ffd).
Method: [untangle-codebase](https://github.com/caldoai/skills/blob/main/skills/untangle-codebase/SKILL.md).
Optional bulk comment/test cleanup was not selected. Existing test assertions and attribution were retained.

## Design decisions and resolved problems

| ID | Design implemented | Problem resolved |
| --- | --- | --- |
| U1 | One private Core HTML encoder, using the existing single-pass implementation. | Five renderers independently maintained the same five-character mapping. |
| U2 | One Rust-bridge HTTP executor; protocol callers specify their byte limits. | Two copies of marshal, request, bounded read, status mapping and strict JSON decoding; a report-only condition inside general transport. |
| U3 | One Web CLI-header template factory. | Channel overrides and affinity settings separately maintained the same Codex/Claude allowlists and builders. |
| U4 | Signal game owns its engine, shared panel, CSS and tests. | The game imported its own engine and panel from authentication UI; authentication now consumes the game feature. |
| U5 | One SQL literal encoder for seven offline legacy planners. | Seven copies of identical NULL, boolean, integer and quoted-text encoding. |
| U6 | One Go module credential adapter. | Community, acquisition and operations separately cloned requests and translated the same user credential header. |
| U7 | One captured currency formatter in the existing currency module. | Wallet and payment hooks duplicated eight conversion/format methods. Their preference and localization state remain separate. |
| U8 | One post-serialization Go-compatible JSON escaper. | Builtin fingerprints, announcement revisions and tool receipt verification independently maintained the same five mappings. |
| U9 | Audio charging implementation lives under billing/charging/engine. | A charging child module was physically stored beneath model_http/audio and reached through a cross-directory path attribute. |
| U10 | One native request host owns admission and draining. | Native model APIs and System One duplicated semaphore/transport setup and shutdown polling. Their caps remain four and two. |
| U11 | Four immutable pricing modules live in billing. | Durable charging and relay contracts depended on HTTP module paths for tool, text-request, grounding and image prices. |
| U12 | Removed three unused private Go functions and the terminal-settlement pass-through. | Functions had no callers, or only delegated unchanged arguments to the actual implementation. |

The price and audio moves relocate 2,001 existing lines unchanged. The Signal engine and CSS also retain their exact bytes. These moves are not counted as deleted implementation.

## Preserved contracts

HTML attribute escaping remains distinct from SVG XML escaping and text-only escaping. JSON serialization, object order, number spelling, size checks, digest construction and error mapping stay with their existing callers; only the identical Go escaping step is shared.

The Rust bridge preserves its authorization headers, HTTP status mapping, strict JSON decoding and lack of automatic retries. The terminal-proof action allowlist remains closed and does not forward a user credential.

| Bridge request family | Request bytes | Response bytes |
| --- | ---: | ---: |
| Native v1 | 16,384 | 65,536 |
| Ordinary v2 | 32,768 | 262,144 |
| Report listing | 32,768 | 4,194,304 |
| Terminal proof | 2,850,000 | 65,536 |

Provider templates preserve header names, order, exclusions, operation shape and independently mutable arrays. The game keeps its algorithm, fixtures, panel markup, clock behavior and styling. Currency conversion arithmetic is unchanged; returned formatter closures retain their captured denomination and rates.

SQL extraction preserves generated literal bytes. Financial interpretation, JSON policies, schema, transactions and database credentials remain in their existing owners. The Go credential adapter preserves the original request and remains inside OperationsGuard.

Pricing modules now use billing Rust paths. All located in-repository consumers were updated. The Core crate is not published; the pinned CLI has no dependency on lmm-core. HTTP and serialized price contracts did not change.

## Verification results

These are developer self-tests; independent acceptance was not run.

| Check | Result and scope |
| --- | --- |
| Python planner regressions | 27 passed, 0 failed. |
| SQL baseline comparison | 7,105 old/new literal comparisons across seven planners matched. No database connection. |
| Web targeted tests | 32 passed, 0 failed across eight isolated Bun processes. Thirty unrelated payment cases were filtered out. |
| Web production build | Exit 0; Rsbuild reported 20.5 seconds. |
| Initial bundle gate | Exit 0; 559,812 gzip bytes against a 786,432-byte limit. |
| Web full typecheck | Incomplete: both attempts exited 137 without type diagnostics. The second used GOMAXPROCS=2 and GOMEMLIMIT=768MiB. No further retry was made. |
| Go webcompat | Exit 0; 15 top-level tests, 45 passing entries including subtests. |
| Go request executor | Native/legacy/terminal wire contracts and exact report boundary tests passed using an in-process HTTP RoundTripper and the real Adapter executor. |
| Go Unix-socket suite | Compiled, then failed at local socket creation: socket: operation not permitted. Existing socket tests remain unchanged. |
| Sandbox escalation | Rejected by the granular approval policy because sandbox_approval is disabled. No alternative network-permission path was used. |
| Core library | Single-job, locked cargo check --lib passed in 5m38s. One existing unused probe_channel warning remains. |
| Core HTML / Go JSON | Two direct source-module test binaries passed, one test each. |
| Native host | Actual Host source with production transport-constructor/dns_slots source and actual Tokio passed four admission capacities and graceful draining in a bounded harness. |
| Relocation comparison | Four pricing files, audio charging, Signal engine and CSS matched their baseline bytes. Panel/clock body changed only in export naming and imports. |
| Formatting | New helpers and corrected imports passed. Existing Core formatting differences remain in config.rs and lib.rs; an existing Web import-spacing warning remains in the parity test. |
| Documentation | Maintained document paths, skill metadata and README badge checks passed. |
| Diff / dependency files | Diff whitespace check passed; dependency manifests and lockfiles unchanged. |

The first new report-boundary fixture used an empty object despite the existing requirement for a rows field. Correcting the fixture to a valid empty report allowed the unchanged exact-limit assertions to pass. Its failure and corrected run are retained in the evidence logs.

### Commands

Python and Rust checks ran from the repository root:

~~~sh
python3 -B scripts/check-docs-brand.py --docs-only

PYTHONPATH=scripts python -B -m unittest \
  test_legacy_sql test_legacy_cutover_keys test_legacy_cutover_money \
  test_legacy_cutover_channel_identities test_legacy_discount_facts \
  test_legacy_money_features_import test_legacy_referral_import

CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 \
  cargo +1.99.0 check --locked --manifest-path services/core/Cargo.toml --lib

rustc +1.99.0 --edition=2024 --test services/core/src/html.rs -o ../core-html-tests
../core-html-tests
rustc +1.99.0 --edition=2024 --test services/core/src/go_json.rs -o ../core-go-json-tests
../core-go-json-tests
~~~

Web checks ran from apps/web with Bun 1.3.14 and Node 24.22.1. Each test file ran separately with bun test --preload ./scripts/test-preload.mjs --timeout 15000:

- src/lib/cli-header-templates.test.ts
- src/features/signal-game/engine.test.ts
- src/features/signal-game/parity.test.ts
- src/features/signal-game/store.test.ts
- src/lib/currency.test.ts
- src/hooks/use-wallet-currency.test.tsx
- src/features/wallet/components/wallet-payment-clarity.test.tsx
- src/features/signal-game/panel.test.tsx

The payment selector was: applies explicit (USD|CNY) bridge rates|one Credit survives currency switches|keeps Chinese payment units.

~~~sh
node ../../node_modules/@typescript/native-preview/bin/tsgo -b
GOMAXPROCS=2 GOMEMLIMIT=768MiB node ../../node_modules/@typescript/native-preview/bin/tsgo -b
RAYON_NUM_THREADS=2 NODE_OPTIONS=--max-old-space-size=768 bun run build
node scripts/check-initial-bundle.mjs
~~~

Go checks ran from services/extensions with Go 1.27.2 and GOMAXPROCS=2:

~~~sh
go test -p 1 ./internal/modules/toolmarket/rustbridge
go test -p 1 -v ./internal/modules/toolmarket/rustbridge \
  -run '^(TestRequestWireContracts|TestReportResponseLimit)$'
go test -p 1 -v ./internal/modules/toolmarket/rustbridge \
  -run '^TestReportResponseLimit$'
go test -p 1 -v ./internal/webcompat \
  -run '^(TestModuleUserCredentials|TestAuthenticator.*|TestOperationsPublicScriptsAndUnavailable|TestOperationsFreshAdminAuthorizationAndModuleAdmission|TestOperationsStrictRequestValidation|TestOperationsGuardPrecedesOuterMuxAndDoesNotBroadenRouting|TestMountOperationsSharesAdmissionAndGuardsBothEntries|TestMountMissingOperationsIsUnavailableNotHost404)$'
~~~

## Remaining structural work

The changed areas were searched again after implementation. Two broader Core findings remain open, so this record does not declare the whole codebase fully untangled.

1. **Usage and dashboard receipt interpretation.** The readers duplicate receipt interpretation but support different states, density limits and loading strategies. A coherent shared validated representation needs isolated PostgreSQL regression checks for both readers. This refactor does not change either reader's financial decisions.
2. **Audio contracts and replay.** Audio price, receipt and integrated arithmetic form a cohesive group. Charging recovery also depends on pure journal state embedded in the HTTP streaming module. The next extraction should move this group with the pure journal validator and verify live processing/replay equivalence. Moving individual structs would retain the underlying inverse dependency.

Real Unix-socket acceptance and a completed Web full typecheck also remain open. No production deployment, live migration, provider call or payment was performed.

## Commits

| Commit | Change |
| --- | --- |
| [2aa785e](https://github.com/TokenNotIncluded/lmm-api-source/commit/2aa785edde6aac6dd915fe29fd580dc0db8e1609) | Decision inventory before implementation. |
| [65231f7](https://github.com/TokenNotIncluded/lmm-api-source/commit/65231f7306793c4117941de03f3c3c395e87bcd7) | Shared SQL literal encoding. |
| [62ca285](https://github.com/TokenNotIncluded/lmm-api-source/commit/62ca2859cef5adb65664582cd2bfc0cdb6868ca1) | Go transport, module credentials and unused helpers. |
| [acfd60a](https://github.com/TokenNotIncluded/lmm-api-source/commit/acfd60ae99cba644559750ebcdf420d6c5717933) | Web templates, currency formatting and game colocation. |
| [7a63d1c](https://github.com/TokenNotIncluded/lmm-api-source/commit/7a63d1cfbc71ea14ed8446cba4bd45752c996ffd) | Core contracts, encoding and native host. |

## Evidence review

- **Contribution:** concrete duplicate owners and reverse dependencies are identified in the decision table.
- **Clarity:** each change has one owner and an explicit behavior boundary.
- **Measured effect:** source ownership and repeated implementations improved; no runtime speedup percentage is claimed.
- **Coverage:** passing checks, environment failures and unrun acceptance are recorded separately.
- **Design:** no new framework, protocol mode, feature flag or compatibility wrapper was added.
