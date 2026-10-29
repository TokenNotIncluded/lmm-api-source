# Frozen-library channel authorization development harness

`authorization_harness.rs` is a standalone `rustc --test` binary that links the
provided non-test `liblmm_core.rlib` and invokes its public `channel_http::router`.
It does not import the library's private `admit` or its `#[cfg(test)]` leaf. Its
synthetic imported facts and policies do not validate the actual import workflow.

Wait for the owner to finish the freeze and provide the Core, rlib and source
pins SHA-256 values. Inspect target filesystem space and similarly named residue
with the space-first procedure before starting. The runner requires a new direct
child of `~/Documents/Codex`, checks the stated hashes, all source/cache pins,
the successful library/main/Clippy command records and exact scoped source, and
rechecks all bindings after the test. It refuses a missing permission integration.
The runner acquires `/tmp/lmm-delegate/heavy-build.lock` and refuses any cgroup
other than 2 GiB memory max, 1536 MiB high, 4 GiB swap max and one CPU quota.

Run under a fresh delegated scope with owner-supplied values (do not reuse an
existing output directory):

```sh
systemd-run --user --scope \
  -p MemoryMax=2G -p MemoryHigh=1536M -p MemorySwapMax=4G -p CPUQuota=100% \
  python3 services/core/src/channel_http/run_authorization_harness.py \
  --freeze /absolute/path/to/freeze \
  --output "$HOME/Documents/Codex/NEW-PRIVATE-CHANNEL-EVIDENCE" \
  --core-sha CORE_SHA256 --library-sha RLIB_SHA256 --source-pins-sha SOURCE_PINS_SHA256
```

The runner creates PostgreSQL 18 with network listening disabled, a private Unix
socket and an empty synthetic database; the test creates a non-superuser role
with only the SELECT grants needed for its identity and directory reads. It
checks five public channel reads, policy and identity rejection, a real directory
read for the opposite permission case, policy installation, unsupported writes,
and table/sequence digests. On success it stops PG and deletes this run's PGDATA;
on failure it stops PG and retains PGDATA and private logs. Failure to stop PG is
reported and its socket is preserved for recovery. Never point the harness at an
existing cluster, even a localhost cluster.

`PASS` covers this developer harness and the bound Core library only. It is not
the library's `authorization_tests.rs`, an actual Native HTTP process, a browser,
an import verification, independent acceptance, or full channel CRUD.
