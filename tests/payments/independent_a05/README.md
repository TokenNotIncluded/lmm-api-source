# Independent A05 process acceptance

These are independent synthetic-payment cases for the frozen base `84100a7` + payment overlays (manifest `2317680de66748510c3f3ef50f735fe42ae684b4c22a3774f2edf00505d0f929`). They use real PostgreSQL, the actual Core main, and separate Go payment-module worker processes. They do not certify the Go host scheduler, live providers, production accounts or financial settlement.

The two `.go.fixture` files are copied into a **private copy** of `services/extensions/internal/modules/payments/` as `a05_independent_test.go` and `a05_automatic_independent_test.go`. Product source must match the frozen manifest. The historical harness is preserved byte-for-byte, including two known failing fixture assertions in its unselected cases: a 15-character hold key and an incorrect `held` expectation after provider submission. Do not run all its cases or silently edit it. The new automatic case uses valid keys, asserts `awaiting_evidence` plus exact held funds, and uses only `Work` after B failure. It preserves accepted proof/key/bytes, actually waits at least 302 monotonic seconds, interrupts a Go receipt update after observing Core commit, restarts the worker/Core, and checks exact ledger legs and no duplicate settlement. The explicit old-inbox fixture edit and synthetic prior consumption are recorded; neither is production data.

Build complete Core library/main and the Go module test executable from frozen inputs; bind all dependencies, source and binaries by SHA256. All heavy builds must use `flock /tmp/lmm-delegate/heavy-build.lock`. The existing v2 resource policy is unchanged. This new run uses the separately authorized 2 GiB admission gate with 2 GiB memory maximum, 1.5 GiB high, 4 GiB swap and one CPU in a systemd user scope. Shared Rust 1.99 cache is read-only. No CI is required. Go main from a source copy needs `-buildvcs=false`; record the frozen manifest independently rather than inventing VCS metadata.

Example Go compilation inside the private module (choose your actual paths):

```sh
flock /tmp/lmm-delegate/heavy-build.lock go test -mod=readonly -p=1 -c -o /absolute/private/independent.test ./internal/modules/payments
```

`run.py` never builds or changes product source. Run separately for `--case automatic` and `--case binding`, using fresh short output directories. Pass absolute paths for `--source-root`, `--compiled-inputs`, `--core-binary`, `--test-binary`, and `--output-root`. It owns only the isolated cluster it initializes, uses the frozen production runtime-grant script (including identity v6 email permissions), serializes with the shared lock and retains failures. Use a bounded systemd user service as above; the raw case may last about 6 minutes. PostgreSQL data is stopped and retained for inspection; after exporting necessary evidence, clean only owned disposable data. Keep tokens and raw snapshots private.

Independent execution evidence and result are reported separately from this reproducibility fixture. Current exact executed launcher and failure history are in the private evidence directory, not inferred from this portable wrapper.

Concrete runner invocation (replace the placeholders with frozen paths and recorded hashes; `OUT` must not exist):

```sh
systemd-run --user --wait --pipe \
  --property=MemoryMax=2048M --property=MemoryHigh=1536M \
  --property=MemorySwapMax=4096M --property=CPUQuota=100% \
  --property=KillMode=control-group --property=RuntimeMaxSec=900 \
  python3 -B tests/payments/independent_a05/run.py \
  --source-root "$SOURCE" --compiled-inputs "$MANIFEST" \
  --core-binary "$CORE" --core-sha256 "$CORE_SHA256" \
  --test-binary "$TEST" --test-sha256 "$TEST_SHA256" \
  --output-root "$OUT" --case automatic
```

Set `OUT` to a fresh short private path, for example `/tmp/a05-accept-unique`; do not reuse an old attempt. The binding case uses `--case binding` and a different fresh output root. Its selected historical test is `TestIndependentA05/binding_capacity_cross_entry_concurrency`; the automatic case selects only `TestIndependentA05Automatic`. The original failing historical over-refund/joint tests are intentionally excluded, without editing their assertions.

For an affected-code replay after an equivalent `check_refund_recovery` refactor, run `--case binding` and `--case over-refund` in **separate fresh databases/output directories**. The second selects `TestIndependentA05Automatic/both_successes_over_payment_remain_fenced` with all its assertions intact; neither short case runs or claims the 302-second clock test. Record new Core hash and its source/feature graph separately while preserving the original Go worker hash.
