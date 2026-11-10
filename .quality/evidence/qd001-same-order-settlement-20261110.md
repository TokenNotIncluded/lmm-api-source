# QD-001 same-order concurrent settlement — component acceptance

Result: PASS. This is a real local Core main + Go host HTTP + PostgreSQL ledger acceptance of one order's concurrent settlement. It is not final release acceptance and does not close QD-001.

Inputs
- Core executable: `/home/lightjunction/.cache/lmm-sub01-validation/lmm-core-core59-initial`
- Core SHA256: `07930fa8cdcfb98a646fb52d7bc92a27f13a135e65c8dda1eb0eabf2fcf4b842`
- Core builder receipt: `/home/lightjunction/.cache/lmm-sub01-validation/core59-initial-artifact.json`; build log: `/home/lightjunction/.cache/lmm-sub01-validation/core-bin-build.log`.
- Core before/after 810-file source manifests are byte-identical, SHA256 `0dbec7d4d565048b1facf67b6793af3a9ccefe6198a9a66acb10cd6295278c52`.
- Go executable: `/home/lightjunction/.cache/lmm-host-component-se2cyucc/lmm-extensions`
- Go SHA256: `b68c87917ccb8032dadcec30fb3c5fb4bbd319bac26c82ff0a07568134cd2105`
- Go archive source commit: `ec3210a27b9bbe35665b7a583c5706a4ea186fd9`; all 737 manifest entries verified against the private source archive. Build receipt: `build-02-result.json`, Go 1.27.2. The earlier VCS stamp build failure remains in its original delivery directory.
- Harness SHA256: `5d5d87a9fbf6745814cd91d4ad6270be916c8c5aa1961fbef89155738b40d659`. Actual executed per-attempt source snapshots and helper hashes are preserved. No Rust/Go compilation, Git/product/docs mutations, production or paid calls were performed.

Source limitations
The frozen Core build has four later live-source EPay changes, expressly allowed by root for this component run:
- `contracts/proto/lmm/core/v1/payments.proto`
- `services/core/src/subscriptions/epay_rpc.rs`
- `services/core/src/subscriptions/external_epay.rs`
- `services/core/src/subscriptions/external_epay_tests.rs`
All other 806 recorded Core/proto input files matched before and after execution, including the commerce/funds/identity implementations and schema used by this slice. The source manifest binds an uncommitted build snapshot at the stated Git base; this is not a claim that the live tree equals that snapshot. The Go archive excludes later SUB/APP changes. Replay this harness on the final combined frozen Core/Go inputs before final release acceptance.

Actual scenario and oracle
- One isolated PG 18.6 cluster, distinct Core/store databases and unprivileged runtime roles. Go cannot CONNECT to the actual Core DB (SQLSTATE 42501 probe). Native Core main `init-db` and `bootstrap-user` set up synthetic users; native schema-manifest contains 59 components.
- Artificial local opening balance 500000 credits, fixed-credit order price 5000. One serial pay and fulfill establish the single test order; no concurrent-payment, unrelated-order, refund or broad suite case.
- Two independent Go HTTP clients POST `/extensions/v1/store/orders/change` with the same shop/order/action `settle`. While holding only that original Core order row, both HTTP requests are sent and unfinished, and two actual Core runtime sessions wait on the lock. No settle retry.
- Both HTTP statuses 200, JSON bodies identical, original order settled. Exactly one reserve-parent capture journal and one Core settle operation. Core/Go durable receipt key/hash/reference/outcome agree on `commerce:journal:3`.
- Buyer available 495000, reserved 0; seller available 5000, reserved 0; both debt buckets 0. Every ledger journal has at least two entries and zero net delta.
- Actual `/proc/PID/exe` digests and PID/start-ticks/boot/device/inode identities agree at spawn, ready and after. Recorded timestamps prove both requests overlap and both return after lock release.
- Successful fixture removed after both owned product processes were reaped.

Results and commands
See `/home/lightjunction/.cache/lmm-predeploy-validation/qd001-settlement-20261110/commands.txt` for exact commands and exit codes. `preflight-01/result.json`: READY, no DB/host started. `attempt-03/result.json`: PASS. `attempt-03/concurrency.json`, `settle-0.json`, `settle-1.json`, `ledger.json`, and process identity files contain raw evidence. `evidence-audit.json` independently rechecks the retained records without starting another process suite.

All failures retained
- attempt-01: FAIL before order setup because fixture database did not satisfy Go's dedicated storage name policy. Fixed only cache setup. Native initialization, logs, original result, source snapshot, and private SQL dumps remain. The exact owned failed cluster was removed only after verified dumps; see `retention-cleanup.json`.
- attempt-02: FAIL because empty owned `core.lock` prevented temporary socket-directory removal. Both business requests and every ledger assertion passed; its original FAIL remains unchanged. Fixed only cache cleanup, then complete attempt-03 passed.
- Native bootstrap stdout and fixture credentials are private local files; do not publish the whole directory.

Actual debt/fullgate gap
`.quality/debt.json` still has QD-001 open, critical, expiry 2026-10-24, reviewer pending. The existing fullgate report `/home/lightjunction/.cache/lmm-release-readiness-20261110/report.json` records `ValueError: expired debt/exception: QD-001` before checks; no new fullgate/build/broad suite was launched. Historical concurrent-settlement runs do cover the same-order case, but their Core commerce/test/schema source hashes drifted, so they did not close the current fixed-input gap. This run supplies the missing real same-order component evidence. Other QD exit criteria and final combined candidate acceptance must be reviewed separately. No deadline, severity, assertion, debt status or reviewer was changed.

Resource status after the slowdown instruction
All test Core/Go PIDs 265763/266018, 272227/272388 and 276299/276598 were absent in `ps`; the failed fixture PID 264041 was also absent. Session 84614 exited 0 after the already-running 1.1 MB failure-dump/owned cleanup. No live build/heavy task belongs to this worker. Only light cache reporting continued.

Review proposal
`evidence.patch` proposes adding this evidence note under `.quality/evidence/`; it does not change debt.json or mark the debt resolved. Root can review and incorporate it alongside the remaining exit-condition evidence. Do not treat this component-only result as a closed/full-release decision.
