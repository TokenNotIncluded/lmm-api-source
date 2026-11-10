# QD-001 fixed-component funds recovery — 2026-11-10

Independent executor: Peirce. Result: PASS for the scoped component cases; QD-001 remains open/critical. This is not final-release or production acceptance.

## Fixed inputs and retained failure

Core service SHA-256: `7f6a0fdc9a5a8ca4c48fa7c05d1f639c09850af4274f0ad4b6c3d2a72d532c00`.
Go service SHA-256: `b68c87917ccb8032dadcec30fb3c5fb4bbd319bac26c82ff0a07568134cd2105` (source `ec3210a`). This is the same pair as the independent ASSIST actual-process evidence, not the current source combination.

The first attempt stopped before PG/services because five live Core source files differed from the original manifest. Failure evidence remains in `~/.cache/lmm-predeploy-validation/qd001-funds-7f6a-b68c-20261110-once01/`.

A read-only cache snapshot exported from `3579ac71f112894e034d1c489ebb80bd4934b9f9` matched all 810 original Core input hashes. Only `core.source_root` changed in a new input file; the binaries, runner, assertions and drift allowance did not change. Snapshot proof: `~/.cache/lmm-predeploy-validation/qd001-funds-7f6a-b68c-once02-inputs/snapshot-proof.json`. No live source was reverted.

## Actual execution

Successful evidence: `~/.cache/lmm-predeploy-validation/qd001-funds-7f6a-b68c-20261110-once02/result.json`, accompanying HTTP/ledger snapshots and execution report. Actual isolated PostgreSQL, Core and Go processes ran the original 33 assertions plus funds permission and recovery cases:

- Unauthorized/insufficient-authority requests returned 401/403/403 without financial side effects.
- Two concurrent HTTP settlement requests returned the same successful receipt with one capture and one settle operation. Concurrency evidence is two pending HTTP requests and one observed Core lock waiter; it is not a claim of two observed lock waiters in this run.
- After settlement, buyer available/reserved was 495000/0 and seller available/reserved was 5000/0; ledger imbalance count was zero. Serial settlement replay did not change this state.
- A real Core refund committed while an injected Go acknowledgement-persistence failure left the Go operation pending and returned HTTP 202. Retrying the original refund again returned 202 without another Core financial effect.
- Removing the injected fault let the service's receipt-only recovery worker mark the original operation complete and order refunded. Core journals and balances did not change during recovery. Final buyer available was 500000, seller available 0, with no reserved funds.

Session 60764 exited 0. Runner 424717, Core 426411, Go 426423 and private PG 424799 were confirmed absent; the private container was removed. Root independently inspected the result and process state. No production account, real paid provider request or production data was used.

The current SUB/APP changes, final fixed combination, browser behavior, remaining full QD exit conditions, release and deployment remain unproven by this component result.
