# Codebase untangling — 2026-11-06

This record covers the original twelve structural decisions across Core, Extensions, Web and offline import tooling, followed by three Core continuation decisions. The [continuation results](#continuation-results--2026-11-06) describe the current source changes and verification limits. The earlier results below remain historical evidence; their successful Core check does not establish a successful build of the continuation.

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

## Original-pass verification results

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

## Follow-up findings from the original pass

The original pass identified two broader Core findings. Both are addressed by the continuation below; they are retained here to explain its scope. This record does not declare the whole codebase fully untangled.

1. **Usage and dashboard receipt interpretation.** The readers duplicated receipt interpretation but supported different states, density limits and loading strategies. A coherent shared validated representation required PostgreSQL regression checks for both readers; U13 implements that boundary and corrects two previously inconsistent terminal-evidence decisions.
2. **Audio contracts and replay.** Audio price, receipt and integrated arithmetic formed a cohesive group. Charging recovery also depended on pure journal state embedded in HTTP streaming. U14 moves that group together and checks live processing/replay equivalence, removing the inverse dependency.

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

## Continuation results — 2026-11-06

Baseline: [a399631d65a21e5e443bcd52d54e3cc7941da0c5](https://github.com/TokenNotIncluded/lmm-api-source/commit/a399631d65a21e5e443bcd52d54e3cc7941da0c5).
Code result: [16c0e225f40d4cf38cc0e90802c2534cb1efcac9](https://github.com/TokenNotIncluded/lmm-api-source/commit/16c0e225f40d4cf38cc0e90802c2534cb1efcac9).

The continuation implements U13–U15. Decisions were recorded before source edits in [527726b](https://github.com/TokenNotIncluded/lmm-api-source/commit/527726bba9bb3935bf353b044c14d0ca2ff8d40d) and [acd0566](https://github.com/TokenNotIncluded/lmm-api-source/commit/acd0566699da81afc2196556941f1096d531592b). Optional bulk Comments and Tests cleanup remained unselected. Existing assertions were retained, and no dependency or lockfile changed.

**Validation status:** the affected source modules passed the bounded checks below, including real PostgreSQL receipt comparisons and drawing-session predicates. A complete Core build or check did not finish: all three attempts ended with compiler SIGKILL. Actual Charging/router regressions and independent acceptance remain unrun. Earlier successful checks elsewhere in this document are not results for this source revision.

### Ownership and resulting behavior

| ID | Implemented owner | Problem resolved and behavior boundary |
| --- | --- | --- |
| U13 | `usage/receipts.rs` owns immutable receipt validation, validated request facts and exact amount projection. Usage, channel projection and dashboard consume it. | Two financial readers independently interpreted identities, commands, terminal evidence and refunds. The original usage validator is retained; dashboard now rejects missing typed terminal evidence and contradictory text-request outcomes consistently with usage. Valid receipts retain their observed amounts and projections. |
| U14 | `billing/audio` owns audio price/receipt contracts, integrated arithmetic and pure journal validation. | Charging recovery depended on HTTP streaming internals. Live processing and durable replay now share the journal state machine and packet/byte limits. HTTP owns provider configuration, transport and response/error mapping. |
| U15 | `image.rs` owns persisted drawing contracts, reference-schema validation and drawing-session authority predicates. | Charging imported metadata and session helpers from HTTP execution, which itself used charging. Billing and HTTP now use the same domain implementation; upload parsing, multipart construction and provider I/O retain their transport owners. |

U13 preserves each reader's authorization, query scope, snapshot handling and error conversion. Dashboard remains settled-only, with its 100,000-request window and separate 2,048-command/refund bounds. Its existing `HISTORY_TOO_DENSE` response remains HTTP 400. Usage continues streaming all receipts for bounded request batches. Invalid shared receipts map to usage storage errors and dashboard unavailability. No charging write, money arithmetic, schema, transaction or runtime database grant changes.

The two dashboard corrections are deliberate behavior changes for corrupt settled evidence. A price calculation can accept empty usage for an unstarted request, so an equal computed price alone does not establish valid terminal evidence. The shared validator additionally requires the tool, text-request and grounding evidence implied by the frozen price and checks that the typed text-request outcome agrees with the final outcome. These checks already existed in usage.

U14 preserves serialized receipt bytes, price arithmetic, journal v3/v4 compatibility and final-packet/EOF ordering. The shared limits remain 16 MiB, 4,096 packets and 262,144 bytes per packet. Each caller retains its own error mapping. Five relocated audio files, including the receipt fixture and existing pure tests, match their baseline bytes exactly. A new fixture check compares live processing with replay.

U15 preserves metadata defaults, schema/version allowlists, legacy stored envelopes, native-session and browser-family meanings, and SQL error classification. Nine moved function bodies match after normalizing whitespace and module qualifiers; both moved SQL predicates are byte-identical. No new runtime configuration or schema is introduced.

Within `services/core/src/billing`, production references to `model_http` decreased from 14 to zero. One test-only reference remains because that test exercises the HTTP image service. This is a measured source-dependency reduction, not a runtime performance claim.

### Continuation verification

These are developer self-checks of the identified changes. Counts below describe distinct test scopes and must not be combined into a claim of full application coverage.

| Check | Observed result | Scope and limit |
| --- | --- | --- |
| Audio source tests | 15 passed; 0 failed or ignored. | Actual audio production files, existing pure tests and the new live/replay fixture check; no HTTP or database recovery execution. |
| Drawing source tests | 2 passed on PostgreSQL 18.6; 0 failed or ignored. | Actual `image.rs`, exact production type/table/trigger excerpts and synthetic rows. Covers metadata/projection and native/browser session predicates, including rotation facts, revocation, foreign families, version mismatch, disabled users and weekly expiry. Does not execute authentication issuance/rotation, financial admission, providers or HTTP routes. |
| Receipt differential cases | 28 passed on PostgreSQL 18.6. | Actual new validator and dashboard adapter compared with both baseline validators using SQLx `PgRow` from synthetic `jsonb_to_record` fixtures. Old/new usage observations agree in every case; valid dashboard observations agree; the two known invalid-terminal cases are rejected consistently. Does not execute Charging writes, application loaders, migrations or HTTP routes. |
| Receipt boundaries | Partial refund remains 49 gross / 20 refunded / 29 net; full refund retains a known zero net amount. Both readers accept 2,048 records; usage accepts 2,049 while both dashboard versions return `TooDense`. | Included within the 28 differential cases, not additional tests. Nonsettled states, canonical-command/identity/source/refund failures, OAuth and sponsorship are also covered. |
| Source provenance | Audio relocations, drawing function/SQL comparisons and receipt source hashes matched. | Receipt harness verifies 35 exact source segments and 34 frozen Rust files. Harness wiring and observation adapters are separate from production validation. |
| Formatting and documentation | Scoped Rust formatting, diff whitespace and maintained-document checks passed. | Only affected paths were formatted; no broad formatting or comment/test cleanup. |
| Complete Core compilation/check | Incomplete: three commands exited 101 after rustc received SIGKILL. | No successful final whole-Core build, metadata check or actual router integration test is claimed. |
| Independent acceptance | Not run. | Module evidence does not replace application acceptance. |

The small harnesses use actual source and already-built dependency artifacts. The successful compile peaks were 243,232 KiB for audio, 253,356 KiB for drawing and 472,208 KiB for receipts. They make the checked module boundaries explicit; they do not establish that the complete service compiles or operates correctly.

The first complete library-test attempt exposed missing test-owned `Change` and `LedgerCommand` imports after extraction. Explicit imports corrected that issue. A later unused production import was moved to its test owner. Subsequent metadata checking still ended with SIGKILL. No further whole-Core retries were made, and the exact cause of the signal was not established.

Harness setup failures are retained with the successful records. Audio initially had a harness module-path error, corrected without production changes. The first receipt run selected a cached `regex` build without required Unicode categories and failed before case execution. Selecting the existing full-feature dependency fixed that harness issue without changing source, inputs or assertions.

### Commands and remaining verification

The following complete-Core commands were attempted with Rust 1.99.0 and one build job. All exited 101 with SIGKILL; none is a passing gate:

~~~sh
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --lib --no-run --message-format=json
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --test usage --no-run --message-format=json
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo +1.99.0 check --locked --manifest-path services/core/Cargo.toml --lib --test usage --test dashboard --message-format=json
~~~

The original dashboard tests and two new actual Charging/router regressions now live together in the automatically discovered `services/core/tests/dashboard.rs` integration target. All original test bodies and assertions are retained; moving the target required only crate imports and formatting, without exposing internals. The new regressions cover terminal-evidence consistency, the 2,048/2,049 boundary and unchanged money-state snapshots. Their actual execution remains pending.

In an environment able to build the complete Core, the affected verification targets are:

~~~sh
cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --test usage --test dashboard
cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --lib billing::audio::
cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --lib model_http::audio::
cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --lib image_drawing_
cargo +1.99.0 test --locked --manifest-path services/core/Cargo.toml --lib audio_charging_variant_keeps_legacy_json
~~~

The evidence archive `44-codebase-untangling-evidence-20261106.zip` records commands, source manifests, results, logs and retained failures. It excludes database credentials, private database files, binaries and the build cache. Its receipt harness README describes the module comparison and the additional workspace prerequisites; the archive is evidence, not a standalone application test environment.

### Continuation commits and review

| Commit | Change |
| --- | --- |
| [527726b](https://github.com/TokenNotIncluded/lmm-api-source/commit/527726bba9bb3935bf353b044c14d0ca2ff8d40d) | Record receipt/audio decisions before implementation. |
| [acd0566](https://github.com/TokenNotIncluded/lmm-api-source/commit/acd0566699da81afc2196556941f1096d531592b) | Record drawing-domain ownership decision. |
| [e33ba97](https://github.com/TokenNotIncluded/lmm-api-source/commit/e33ba9701d028f56ac98c78a249cbda1b1f0977f) | Billing owns audio contracts and journal replay. |
| [a930c31](https://github.com/TokenNotIncluded/lmm-api-source/commit/a930c31117166ad386eb888db47a4b21f7f04920) | Drawing evidence and authority move outside HTTP orchestration. |
| [16c0e22](https://github.com/TokenNotIncluded/lmm-api-source/commit/16c0e225f40d4cf38cc0e90802c2534cb1efcac9) | Usage and dashboard share immutable receipt validation. |

A finite follow-up sweep found no further high-value structural change within this scope. Other inspected Go decoders and SQL encoders differ in validation, null/number handling, bounds or error policy and were left with their existing owners. Inspected Web imports served composition, pure logic or type use. The earlier full Web typecheck and Unix-socket acceptance limitations remain historical open items; this continuation did not rerun those unrelated checks.

The review ties each contribution to a duplicated owner or reverse dependency, names the behavior changes, distinguishes measured source effects from runtime claims, and records coverage gaps beside passing evidence. No new framework, feature flag or compatibility wrapper was added. No production deployment, live migration, provider request or payment was performed.
