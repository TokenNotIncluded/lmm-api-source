# io.net deployment management development evidence — 2026-11-10

Developer self-tests passed for the APP-04 implementation. Independent acceptance, publication and production deployment are not established by these results.

## Implemented behavior

The Go extension and Web UI provide connection settings, deployment list/create/details, container/log reads, renewal/update/rename/delete and original-operation recovery. Settings use revision checks and private encrypted credentials with io.net-specific authenticated context. Existing script-repository credential encryption remains compatible. L5 reads and L6 mutations require fresh Core authorization through the existing interface.

A durable operation key records the original encrypted request and credential before sending. Only the winning insertion dispatches. Duplicate requests read the original receipt; changed arguments conflict. Ambiguous remote outcomes remain UNKNOWN and block conflicting work. Recovery never re-sends a mutation or treats a deployment's existence as proof of renewal. Cancellation creates a tombstone only for an operation not yet admitted. Fresh installation and explicit offline `upgrade-ionet` support the new extension tables; normal startup does not upgrade them.

## Executed checks

- Five Go operations tests passed, including io.net behavior and existing script credential encryption/private HTTP checks. The fresh HTTP authority test passed after correcting a ServeMux route ambiguity. Receipts now use `/api/deployments/operations/receipts/{key}`.
- Four Web API/recovery cases and the existing settings-generation hook case passed. Whole-site Web typecheck passed once. These use a test HTTP adapter, not a real browser/provider.
- Five actual PostgreSQL18 tests passed: io.net settings/receipt/UNKNOWN behavior, offline upgrade rollback/recovery, HTTP permissions/cancellation, and existing script credential persistence/HTTP recovery. Provider traffic uses a local TLS/SSE fixture. No live paid operations occurred.
- The four changed Go entry packages compiled with no test cases selected. The initial live-tree compile encountered concurrent source changes; it is not final whole-package acceptance.

Evidence: `~/.cache/lmm-app04/`, including `manifest.json`, `source-hashes-pg.json`, `pg-first.log`, Web logs, `entry-compile-first.log` and `report.md`. The 35 owned source hashes are frozen. The first invalid launcher command and first genuine route-registration failure remain in separate logs; neither is counted as a pass. The private PG process stopped after testing.

## Provenance and remaining acceptance

Immediately after PG testing, owned files had no drift. The only other change was two new cases inside the unselected `storeLegacyRouteCases` test helper; root inspected the diff, which adds no initializer or package-global state. Later STORE/Stripe edits also affected shared Go dependencies. The recorded results establish the selected behavior, not a current full-package binary fingerprint. Root then verified a read-only snapshot of committed base `8cf8c7a` plus precisely the 35 APP-04 files. All four entry packages compiled successfully with zero test cases; all 791 snapshot files and 35 live owned files matched before/after. Snapshot tree SHA-256: `cc94238c6d8cf6b9a2100829b5efb15d91b7012c4421ebc3b0ddd18bebc27d4b`. Evidence: `release-validation.json` and `release-entry-compile-first.log` in the same private cache. Uncommitted STORE/Stripe changes are not required by this submitted combination.

Still unrun: actual offline CLI PostgreSQL execution, real Core process authorization, browser interaction/visual acceptance, live io.net contract checks and independent acceptance of a frozen production candidate. No claim of release readiness or production migration follows from these checks.
