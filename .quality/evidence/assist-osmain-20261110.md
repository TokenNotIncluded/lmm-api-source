# Assistant actual-process component acceptance — 2026-11-10

Independent executor: Dirac. LIMITED PASS, not final release qualification.

Inputs: Core executable SHA256 `7f6a0fdc9a5a8ca4c48fa7c05d1f639c09850af4274f0ad4b6c3d2a72d532c00` (810 frozen inputs matched committed `3579ac7`); Extensions executable SHA256 `b68c87917ccb8032dadcec30fb3c5fb4bbd319bac26c82ff0a07568134cd2105` (component `ec3210a`). These are developer-built binaries, independently identified and executed, not independently rebuilt or the final combined candidate.

Actual Core main/RPC, Go main/HTTP/SSE and private PostgreSQL 18.6 used a controlled loopback model provider and synthetic fixture credit. The run proved:

- Watch through Go delivered a prefix before the upstream released its final response; each original turn dispatched once.
- Two completed turns each charged the independently calculated 44 credits. Wallet readbacks were 999956 and 999912, reserved zero. Repeated original receipt GET did not change requests, journals, balances or provider count.
- Closing the original POST stream retained its hold. GET on the original turn recovered prefix and final without another POST or provider request.
- Cancel and repeated Cancel left uncertain work in `unknown`/`reconcile`, with 3744 reserved and zero settlement. This is not a refund or completed reconciliation.
- Another login for the same owner received 403; a different owner received 404; revoking the original credential resulted in 401. No output disclosure or new dispatch. These are end-to-end denials, not separate negative direct-gRPC coverage.

The configured native read-only database check passed for models, events, console, Web identity and assistant capabilities. The actual 59-component catalog matched `38df997ce287e5bfe31b05a50cc1c7a831189b719e3690b7127a3d6080ad5640`.

Evidence root: `~/.cache/lmm-predeploy-validation/20261110-independent-assist-osmain/final-attempt08/`. Inspect `report.md`, `receipt.json`, `http-trace.json`, `results.json`, expected-cost-before-release files, frame logs and cleanup records. The executed runner and SQL dependencies are frozen in sibling `frozen-final-r07/`. Session 33957 exited zero; private processes stopped and fixture data removed.

Earlier failures remain: required Web flags, missing SQL include, pool limit, events grants, Go database URL contract, cursor key format, and an incorrect expected price. The final expected price was derived from the fixed quote and rounding rule before releasing provider responses. No product assertion was changed to match an observed settlement.

Before-run binary content hashes passed; after-run file identities/stat metadata were unchanged, without a second full content hash. No browser cookie/CSRF lifecycle, paid external provider, eventual uncertain-provider reconciliation, historical migration, final combined build, release or production deployment was tested. QD-001 remains open; this evidence covers its assistant stream/cancel component only.
