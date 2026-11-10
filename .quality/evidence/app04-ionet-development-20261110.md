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

Still unrun: real Core process authorization, browser interaction/visual acceptance, live io.net contract checks and independent acceptance of a frozen production candidate. No claim of release readiness or production migration follows from these checks.

## Independent actual CLI finding

The independent verifier built source `a32f98c` into the actual Go executable SHA-256 `43bfc986f0b1cf0af1a72e3f96523d60054c16d608fa8017cbb887478c99b924`; its 791 frozen source files did not drift. Actual CLI/private-PG verification is **FAIL**: an otherwise fenced runtime role with database CREATE authority was accepted and the addon committed. The unchanged original module catalog fingerprint did not cover that database-level privilege.

Original operation recovery and re-entry, foreign operation/installer rejection, post-DDL transaction rollback and normal runtime object ACLs passed. The first attempt also exposed a runner-only OID string/integer comparison error; its evidence was retained, and only the unfinished checks were completed with corrected normalization in attempt02. Those passes do not close the product defect.

Evidence: `~/.cache/lmm-predeploy-validation/20261110-independent-app04-cli/summary.json` and `attempt01/product-finding-runtime-create.json`. Both private PG instances stopped and their PGDATA was removed. At that point the implementation fix was awaiting verification. The later targeted result below closes this finding while retaining the original failure.

## Targeted independent fix verification

`APP04-IONET-UPGRADE-RUNTIME-DB-DDL-001` is fixed and independently verified for the offline CLI scope. The verifier froze `1da1c64` plus the two upgrade implementation/test files (791 inputs unchanged), built actual CLI SHA-256 `093566797f14ad7acc811450d76e7060d7673d134b8c1fa1ff5d2943348b3dc1`, and ran once against private PostgreSQL 18.6. The original upgrade regression passed without skipping. All eight effective CREATE/TEMPORARY refusal cases (direct/PUBLIC, install/recovery) preserved catalog, data and receipts; after revoking excess privileges, normal installation and original-operation recovery passed. No retry was needed.

Evidence: `~/.cache/lmm-predeploy-validation/20261110-independent-app04-cli-recheck/result.json`; `attempt01/cleanup.json` confirms private PG stopped and fixture removed. The original failure remains retained. This closes this finding only; it does not establish full APP04, live-provider or production acceptance.
