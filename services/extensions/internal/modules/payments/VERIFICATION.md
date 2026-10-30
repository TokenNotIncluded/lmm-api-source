# Coinbase Business self-test — 2026-10-30

This change adds Business Checkouts to the native wallet on source baseline
`b521f5575bebe4de7195bbd420b16637f46042d5`. Self-tests and the isolated schema
checks below are development evidence; independent acceptance was not run.

| Check | Observed result |
| --- | --- |
| Go payment package and Rust RPC bridge, `go test -mod=readonly -race -count=1 ./internal/modules/payments/...` | Passed on Go 1.26.10. |
| Go composition/app/webcompat payment and Coinbase checks | Passed. |
| Go `vet` for the affected packages | Passed. |
| Native checkout parser/component tests | 9 passed. |
| Web `tsgo -b`, changed-file formatting and lint | Passed. |
| Coinbase UUID key persistence on isolated PostgreSQL 18.6 | 12 concurrent creates retained one checkout job; closing/reopening the DB connection and repeating the request retained its original UUIDv4. |

The Go protocol tests exercise real loopback HTTP with synthetic credentials:
ES256 signature verification, exact method/host/path claims for sandbox and live,
fresh JWTs, exact USDC request amounts, response bindings, permitted checkout
host and unsigned lookup observations. Callback tests preserve original bytes,
check the current and legacy signature headers, reject conflicting JSON and
bindings, and require Core proof admission. A simulated timeout followed by a
new Service instance preserves the original key and stops a second provider
POST. This last test uses the existing memory fixture; the separate PostgreSQL
test covers persisted key identity, not a full Rust-ledger payment.

The first Go attempt was blocked by sandbox restrictions on local TCP and Unix
sockets. The same tests passed with local-listener permission; assertions were
unchanged. The nested PostgreSQL module initially lacked checksums for the
already pinned Waffo dependencies; its module metadata was synchronized from
the existing offline cache, then the new PostgreSQL test passed:

```sh
cd services/extensions/internal/modules/payments/pgtest
PAYMENT_COINBASE_TEST_DSN='<fresh isolated lmm_payments_test DSN>' \
  go test -mod=readonly -race -count=1 \
  -run TestCoinbasePostgresRetainsFirstUUIDAcrossConcurrentCreates -v .
```

Core's new verifier and offline-upgrade tests are included in source. Full
Core compilation/tests were blocked by the unavailable pinned `oxide-auth`
Git dependency in this network-restricted environment. Rust formatting and
isolated PostgreSQL schema checks are recorded in the
[Coinbase upgrade guide](../../../../../docs/schema-coinbase-payments-upgrade.md).
No real Coinbase sandbox account, merchant payment, browser visual acceptance,
production upgrade or deployment was exercised.

# Verification record — native wallet gateways, 2026-10-30

This change starts from source
`6523c4b4b831643d69de3874c2e35933c91bfc2b` and compares the wallet channels with
legacy source `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`. The implemented scope is
native wallet top-ups through Stripe, EPay, Creem, Waffo Global and Waffo
Pancake, including Alipay, WeChat Pay and Linux DO Credit EPay presets. FastPay
is rejected both as a provider and as an EPay method. See
[the gateway guide](../../../../../docs/payment-gateways.md) for configuration,
protocol ownership and the remaining subscription/store/refund scope.

These are developer self-tests. Independent acceptance, real merchant
integration and production deployment were not run. No actual payment
credentials, real charges or production records were used.

## Checks on the final payment behavior

| Area | Observed result and scope |
| --- | --- |
| Rust provider and existing payment units | 35 passed. The one database-dependent test was initially ignored, then explicitly executed separately against isolated PostgreSQL. Original signed bytes, wrong bindings, duplicate proofs, EPay GET/POST and FastPay rejection are included. |
| Core payment configuration | 3 passed, covering the existing Stripe contract, v2 per-channel policy and mandatory private Global inquiry configuration in both environments. |
| Fixed upgrade contract units | 5 passed against the corrected predecessor/successor manifest and catalog pins. |
| Real Core/PostgreSQL top-up regression | 1 passed, containing five profiles: EPay CNY, EPay LDC, Creem, Global and Pancake. Each executes actual Core prepare, independent evidence verification, ledger credit, duplicate recovery and cross-intent rejection. Global uses a test-only local inquiry response. The real site-statistics reader reports LDC separately under virtual units, with the expected five orders and exact ledger total. |
| Go adapters and host | Payment module tests passed after the adapter implementation; changed Creem/Global/catalog cases and the final EPay/POST/FastPay cases were rerun after their respective changes. Composition, app mounting/callback transport, web compatibility, controlled RPC deadlines and the native UDS bridge checks passed. The final private `/orders` response also preserves the stored POST method. |
| Generated protocol | `scripts/generate-core-protocol.sh --check` passed after the final `EPayCheckout.method = 4` change. |
| Web | 7 focused tests passed after the final POST form change; scoped lint/format and the full TypeScript check passed. The production Web build passed before that final form change; the bundler was not rerun afterward. |
| Local edge routing | The actual nginx fixture passed 17 checks, including payment-prefix precedence, browser Bearer preservation, quote/checkout routing and GET/form callback transport. Fixture upstreams do not establish a real joint Core/Go/browser flow. |

Rust used 1.99.0 with the checked-in lockfile and a bounded build environment
(`CARGO_BUILD_JOBS=2`, incremental compilation off, development/test debug info
off). Go used 1.26.10 with the checked-in module graph; SDK modules were not
replaced by local copies. Web used the existing installed, locked dependencies.
The final non-test development Core executable built successfully with SHA-256
`c9773845270626fc8bd6b3c5ca483ece6dccbd6d54b3fbc5597684637412c0ad`.

The final Rust library run was `cargo test --offline --locked --lib payments::
-- --nocapture`. The compiled test executable then ran the filters
`config::tests::payment_configuration`,
`core_schema::upgrade::payment_gateways::tests`, and the explicitly enabled
`payments::providers::tests::postgres_gateway_proof_credit_and_personal_payer`
with `--exact --ignored --nocapture` and a private fixture `DATABASE_URL`.

Go checks were scoped to:

```sh
go test ./internal/modules/payments -count=1
go test ./internal/composition ./internal/webcompat ./internal/app ./internal/coreclient
go test ./internal/coreclient ./internal/modules/payments/rustbridge -count=1
go test ./internal/webcompat ./internal/modules/payments/rustbridge -count=1
go test ./internal/modules/payments -run TestHTTPNeverExposesEvidenceOrInternalJobs -count=1
go test ./internal/modules/payments ./internal/composition \
  -run 'TestCoreEPayKeepsMD5KeyOutOfGoAndPreservesLDC|TestPaymentV2RestoresLegacyProvidersWithoutFastPay' -count=1
```

These commands ran at the corresponding implementation stages; they are not a
claim that the full repository test suite or all fixture-dependent joint tests
ran on one final snapshot. After the final HTTP-method changes, their affected
tests were rerun. Existing Stripe restart/refund acceptance below remains a
historical result and does not certify restart or refund recovery for the new
protocols.

## Actual fixed-predecessor upgrade

The final executable above passed
[`tests/payment_gateways/upgrade_selftest.py`](../../../../../tests/payment_gateways/upgrade_selftest.py)
against private PostgreSQL 18.6 UTF-8 fixtures. The script reconstructs the
exact predecessor from Git source `6523c4b`, checks each old SQL component
against the compiled manifest, and invokes a frozen copy of the actual new
Core CLI. It does not claim to have run the old executable's `init-db`.

The run observed the 28 → 29 component transition, 175 → 176 tables, all 22
existing sequences preserved, and all 7,068 successor catalog definitions
matching the compiled target. Synthetic old rows and original permissions
were unchanged. The exact original request returned `already_committed`;
changing the same request or selecting a new upgrade ID was rejected.
Predecessor catalog drift, stored-receipt drift, unsafe default ACLs and an
existing transaction reused across channels were rejected. The unsafe-default-
ACL attempt rolled back completely. The upgrade receipt rejected mutation.

The actual `check-db` command accepted the restricted runtime after upgrade,
rejected table-level and column-level SELECT grants on the private receipt,
and accepted the restored restricted grants. All cases used the same Native
SHA-256 above. The run is recorded as `lmm-payment-upgrade-selftest-r3`; it is
a developer self-test, with no independent acceptance, real lost-COMMIT-ACK
injection or existing-site writer/fence/deployment validation.

The command shape was:

```sh
python3 tests/payment_gateways/upgrade_selftest.py \
  --core services/core/target/debug/lmm-core \
  --psql /tmp/lmm-boundary-seal-runtime/root/usr/lib/postgresql/18/bin/psql \
  --host /tmp/lmm-payment-pg --port 55439 --admin lmm_payment_admin \
  --library-path /tmp/lmm-boundary-seal-runtime/root/usr/lib/x86_64-linux-gnu \
  --output /workspace/scratch/ae9ee2e44cd8/lmm-payment-upgrade-selftest-r3
```

## Preserved failures and validation limits

The first unconstrained Rust build was killed for memory use, and the first Web
typecheck exited 137; their bounded reruns passed. Initial Go composition
fixtures omitted the required Pancake tax category and expected Go to validate
an endpoint owned by Core; those fixtures were corrected. A host test still
expected GET callbacks to be blocked; it was updated to the explicit GET/POST
ingress contract, without treating the downstream fixture's rejection as a
successful payment.

The first schema-target generator omitted `image_trust_extension.sql` from the
existing concatenated trust-pricing component. The actual CLI's manifest gate
rejected that target before creating any test databases or roles. The generator
and pinned-old-source fixture were corrected, every component/installer/query
hash was checked against the actual compiled manifest, and predecessor and
successor catalogs were observed again in fresh PostgreSQL databases. The
corrected successor manifest matches the executable's stdout byte for byte;
the original failure is retained separately from the subsequent run.
The next run reached the verified predecessor but its test-only ACL snapshot
query had a missing SQL quote. The query was corrected, and the full third
run passed using the same Core executable; this was not a product-code fix.

The initial synthetic PostgreSQL cluster used SQL_ASCII and rejected the
Unicode fixture. Subsequent payment and schema checks use a separate UTF-8
cluster. None of these fixture corrections relaxed provider signatures, money
conservation, immutable intent bindings, catalog pins or runtime privileges.

New-provider process restarts, actual provider accounts, full browser payment
flows, existing-site deployment controls, subscription lifecycle, store
payments and the four added protocols' refunds remain unverified or
unimplemented as identified in the gateway guide. Source presence and these
self-tests do not establish full new-architecture completion.

# Historical verification record — durable proof WIP, 2026-10-12

The current change moves proof acceptance before callback ACK and preserves a
Rust-issued proof reference through Go inbox and jobs. Go no longer holds Stripe
webhook secrets. The initial source base on `wip/rust-core-go-extensions` was
`013aa7df959530be7cb9470c396993c32c7fb7ca`. Results below bind the recorded source
hashes; they do not certify every concurrent change or the entire branch.

## Current local checks

At 2026-10-11 20:38 UTC, using mise Go 1.27.2 with the generated provider-proof
RPC contract and controlled client wrappers:

```sh
cd services/extensions
MISE_IGNORED_TOOLS=rust MISE_AUTO_INSTALL=false mise exec -- go test -mod=readonly -race ./internal/modules/payments/... -count=1 -timeout=90s -json
MISE_IGNORED_TOOLS=rust MISE_AUTO_INSTALL=false mise exec -- go vet ./internal/modules/payments/...
```

Both packages passed: **63 top-level tests, 160 tests including subtests**, zero
failures. Three explicit fixture-dependent tests skipped: the two standalone
mocklab adapters and `TestRustDurableProofAcrossGoRestart`. Vet and scoped gofmt
checks passed. The document gate and its 33 unit tests also passed. This tested
the uncommitted payment-source manifest based on the revision above; it is not
a claim that an untested commit or all repository changes passed.

The tests cover controlled local UDS RPC metadata and proof receipt mapping;
Rust-unavailable and invalid-proof non-ACK; lost inbox write recovery; unchanged
original bytes; stale queued proof lookup; concurrent proof upgrade; and
missing-proof repair of a done inbox with a quarantined credit job. Go state
fixtures remain synthetic and do not prove real Rust journals or balances.

The isolated nested PostgreSQL race test passed at 2026-10-11 20:46 UTC: one
top-level test and six recovery subtests, zero failures or skips. It used only a
new synthetic `lmm_payments_test` database, preserved source hashes and left the
separate joint database untouched. The scenarios covered durable evidence and
proof references, unchanged raw bytes, exact event deduplication, review repair,
a done inbox repairing its quarantined semantic job, and final money-job state
preservation. Its Core/provider fixtures are synthetic; this proves real Go
PostgreSQL storage behavior, not real Rust journals.

## Completed Rust/Go recovery acceptance

At 2026-10-11 21:23 UTC (2026-10-12 locally), the isolated runner passed
`real_go_http_uds_proof_survives_expiry_and_service_restart` in the Rust
`payments` integration target: **one passed, zero failed or ignored**, with
14 other tests filtered out. The runner recorded 330 compiler inputs unchanged
before and after execution. It used mise-selected Go 1.27.2 and pinned Rust
1.99.0, separate fresh synthetic Core/Go PostgreSQL databases, actual Go loopback
TCP HTTP callbacks, and the real UDS client, bridge and Rust payment service.
Provider API responses were simulated; signed webhook bodies were created by
the Rust runner, and no webhook secret or Core DSN was given to Go.

Four separate Go processes executed `TestRustDurableProofAcrossGoRestart`:

| Phase | Observed result |
| --- | --- |
| `recv` | Accepted first webhook and drained its inbox into a credit job, without posting money. |
| `recv-lost` | Rust durably accepted a second event; an injected Go inbox-insert failure returned HTTP 503. |
| `offline` | With Rust UDS stopped, callback delivery returned HTTP 503. |
| `settle` | After a real 302-second wait, webhook-key rotation and reconstruction of the Rust service, old signed callbacks recovered durable proofs and settled once. |

The runner also constructed a historical Go queue shape: an already-drained
inbox and review credit job with missing proof references, while Core already
retained the accepted proof and had no credit receipt. Identical redelivery
recovered that proof, reopened the inbox and repaired the same job. This is a
synthetic legacy queue-state test, not migration of real historical Core proofs.
Duplicate deliveries and 12 concurrent workers preserved **two accepted proofs,
one credit receipt and one journal**, whose entries sum to zero. Go and Rust
both retained the original event/payload bindings.

The Rust runner constructed two service instances in one operating-system
process: **`rust_os_process_restart=false`**. Go processes actually restarted;
the Rust OS process did not. The result does not establish Caddy or browser
acceptance, normal host mounting, real Stripe/ePay interoperability, live
channels, all refund flows, a deployed release or production readiness. Rust's
failed-refund release guard remains in place, and an ePay Rust verifier is still
missing. Neither standalone mocklab adapter test was enabled by this run.

The private local artifacts `checkpoint-06-payment-real-go-joint-receipt.json`
and `checkpoint-06-payment-joint-safe-result.json` record the exact command,
environment, stable input hashes and safe result. The Go payment report binds
those artifacts to its unchanged 17 Go source files. Only the three module
documents changed after the joint run; Go tests were not rerun for these wording
changes.

# Historical verification record — 2026-10-10

## Executed locally

The available local toolchain was Go 1.23.2 on Linux/amd64. The repository requires
Go 1.25.0 and its RPC dependencies were not available in this network-restricted
container. The standard-library payment package was tested without altering the
repository go.mod:

```sh
cd services/extensions/internal/modules/payments
GODEBUG=httpmuxgo121=0 GO111MODULE=off go test -race -count=10 .
GODEBUG=httpmuxgo121=0 GO111MODULE=off go test -coverprofile=coverage.out -json .
GODEBUG=httpmuxgo121=0 GO111MODULE=off go vet .
```

Results: **41 test functions, 71 tests/subtests passed; 10 consecutive race-test
runs passed; go vet passed. Statement coverage: 67.8% for this package.** The
explicit ServeMux setting is needed because module-off mode otherwise selects
legacy routing defaults. CI uses the repository's Go version normally.

The local checks exercised exact signatures and original callback bytes,
merchant/environment checks, stale/future delivery signatures, rotated secrets,
duplicate JSON/form fields, concurrent duplicate events, semantic duplicates,
provider transaction uniqueness, wrong amount/currency/account binding, explicit
account ownership, core outages, ambiguous or invalid receipts, callback-before-
order recovery, refunds before credit, partial/cumulative refunds, lost hold and
commit responses, no timeout-based release, provider retry-window expiry,
unsigned reconciliation, pagination limits, an in-flight evidence upgrade,
credential redaction, HTTP request validation and private-network address denial.

All provider calls used an in-process fake HTTP transport. No real Stripe/ePay
account, real funds, private credentials or production service was used.

## Added but not executed in the local container

- `rustbridge/bridge_test.go`: generated-contract receipt mapping, original
  evidence preservation, controlled Unix gRPC metadata, integer account IDs,
  user/service credential separation and unregistered-service failure.
- `pgtest/postgres_test.go`: actual PostgreSQL concurrent row updates, rollback,
  immutable ownership, unique provider transaction binding, event deduplication,
  restart/reopen persistence and dedicated-database guards.
- `.github/workflows/mk10-payments.yml`: repository-toolchain checks plus a
  disposable PostgreSQL 16 service. Consult the PR's check results for its
  execution status; the local result above is not a claim that CI passed.

## Still requires joint acceptance

The Rust CorePayments service is not registered at the starting base. Thus no
real Rust-ledger end-to-end acceptance, generic independent provider verifier,
prepared-intent read authorization or production host registration was verified.
The unresolved contracts are listed in `PROTOCOL_REQUIREMENTS.md`. Mock receipts
in unit tests prove Go state transitions only; they do not prove Rust journal or
balance correctness. The PR is intentionally Draft and must not be deployed.

## Upstream layout synchronization

During implementation, base commit `4179ea1acfa1172afbd0d16927acb1c3c3a93a10`
renamed `apps/api-go` to `apps/lmm-extensions`. The task branch was rebased onto
that commit and its files, commands and workflow follow the new directory.
The first CI attempts failed before tests: first a health-command quoting error,
then the obsolete Go module path. Both workflow issues were corrected; check the
latest PR run for the actual integration-test outcome.
