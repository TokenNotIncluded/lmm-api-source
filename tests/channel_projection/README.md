# Channel projection independent acceptance

运行状态以对应运行目录的 `result.json` 为准；本文只描述固定验收合同，不记录即时编译或通过状态。Main 负责 Cargo/PG 协调。
All fixture funds, provider responses and historical K evidence are synthetic.
The first stage exercises real funding code and real loopback HTTP; the million
stage proves the untrusted receipt validator/read model, not one million real
funding transactions. It does not prove provider TLS or a real legacy K.

## Fixed evidence matrix

| Stage | Independent expected result |
|---|---|
| Native held at provider gate | dirty empty; pending 1; consumed 0 |
| Two Native settlements | each gross 41/refund 5/net 36; actual wallet 999928 from 1000000 |
| Native exact request/refund replay | HTTP request returns 409 request_conflict with the same request ID (no response cache/redispatch); refund reuses its receipt, provider hit count stays 2; all source ledger and projection rows unchanged |
| Image held/settled/refunded | pending 1; gross 2000/refund 1/net 1999; actual wallet 8001 from 10000 |
| Image exact request/refund replay | same operation, one provider hit; source ledger and derived rows unchanged |
| Product owner rebuild | every source request passes the sole full receipt validator; live and rebuilt rows equal |
| Restricted runtime rebuild | rejected; ready/dirty/rollups/count unchanged |
| Runtime opening integrity | imported, raw/K/offset/source/coverage/missing-parent forgeries reject with trigger P0001; same existing target FK remains unused |
| Legitimate HTTP opening copy | parent facts identical, false coverage stays false; no money writes |
| 1m same-channel + ten sparse | 1000000 channel-1 receipts and ten channel-2 receipts, plus two real seeds; channel-1 lifetime gross 41000041/refund 5000005/net 36000036/settled 1000001; channel-2 gross 451/refund 55/net 396/settled 11 |
| UTC boundaries | synthetic timestamps cycle midnight−1, midnight, midnight+1, midnight+86399, next midnight; each has 200000 channel-1 receipts; inclusive full day has 600000 |
| Exact rollups | independent Rust SHA256 low-six-bit buckets; bidirectional EXCEPT of full contribution sums against all daily and lifetime rollups |
| Million preserve copy | HTTP reset_balance=false succeeds; native own amount 0, inherited net 36000036; later real source refund 1 changes source to 36000035, copy stays fixed |
| Late terminal attempt | all lifetime readers fail unavailable while dirty; unrelated historical-day read remains available; owner rebuild yields candidate-only unknown |
| Missing channel proof | real reserve/start/settle/refund remains valid money, channel 0 global unknown contributes to every queried channel |
| Read-only GET | a separate restricted connection defaults to read-only; recovery/counter GET succeed; no source ledger or projection change |
| Corrupt money receipt | schema-valid unmatched settle command must fail full owner rebuild; derived/ready state rolls back, source ledger unchanged |

The complete raw reservation contract/admission/final/attempt/refund and canonical
ledger-command receipts are cloned from the real Native seed. Every clone gets
new canonical request/operation IDs; no contribution or rollup is directly seeded,
no triggers are disabled, and there is no validator skip flag. Source rows are
written in SERIALIZABLE batches of 1000. The million fixture uses an existing
empty database named by Main and never creates or clears one.

## Paths and wiring

- `services/core/tests/channel_projection.rs`: opt-in integration test; requires
  `LMM_PROJECTION_ROWS=0` for small-only diagnosis or exactly `1000000` for scale.
- `services/core/tests/common/channel_projection.rs`: receipt generator,
  canonical command identities, source ledger snapshots, product rebuild and
  independent exact rollup oracle.
- `services/core/tests/common/channel_projection_image.rs`: private image unit;
  Turing adds the cfg(test) path declaration in `usage/channel_projection.rs`.
- `tests/channel_projection/run.py`: prebuilt binary runner; never invokes Cargo,
  starts PG, applies grants outside fixture code, or tunes a database.

Product API: `core_schema::rebuild_channel_usage(&PgPool) -> Result<u64, IdentityError>`.
The test calls this actual owner API; a dummy subprocess is not accepted.
The shared runtime fixture must load the canonical channel-projection runtime
grants; Main has wired that block. No changes to existing tests are needed here.

## Main's later commands (not run during staging)

After Main freezes sources and compiles both test artifacts serially:

```sh
python3 -B tests/channel_projection/run.py
```

This prints the source digest and fixed plan, without connecting to PG. Main
places the two **prebuilt** test binaries, private URL files and the approval
receipt inside home cache. The integration database must already exist and be
empty. The image test uses the normal SQLx child-database harness, so the receipt
must explicitly authorize that on Main's coordinated isolated server.

Receipt shape (fill exact local paths/hashes; do not store real URLs here):

```json
{
  "isolated_local": true,
  "allow_image_sqlx_database": true,
  "source_files_sha256": "from-readonly-plan-after-source-freeze",
  "lib_binary": "/home/lightjunction/.cache/.../lmm_core-test-binary",
  "lib_binary_sha256": "sha256-of-that-binary",
  "integration_binary": "/home/lightjunction/.cache/.../channel_projection-test-binary",
  "integration_binary_sha256": "sha256-of-that-binary",
  "projection_database_name": "main_coordinated_empty_projection_fixture",
  "projection_database_url_file": "/home/lightjunction/.cache/.../projection-url.private",
  "image_database_url_file": "/home/lightjunction/.cache/.../sqlx-parent-url.private",
  "pg_data_directory": "/home/lightjunction/.cache/.../owned-pg-data",
  "output_directory": "/home/lightjunction/.cache/.../channel-projection-runs"
}
```

Create the output directory in home cache and make the approval/URL files mode
0600. Then, only after Main coordinates the isolated server:

```sh
python3 -B tests/channel_projection/run.py --execute \
  --approval /home/lightjunction/.cache/.../approval.private.json
```

The runner verifies source and binary hashes, confirms the exact test exists,
requires one actual passed test per stage, and refuses zero-test success. It
runs image first, then the Native/small/million test. It checks `df -h`, bounded
`dust`/`prefixlens` inventories and at least 32 GiB free before generation and
again after image. No content is deleted based on those inventories.

Each stage is bounded to MemoryHigh 4250M/MemoryMax 5000M/SwapMax 1536M/CPU 150%
and 7200 seconds. Seven lifetime samples record median and maximum latency;
counters and boundary reads have a fixed 10-second local ceiling, preserve copy
20 seconds. An inline Python `os.wait4` worker records monotonic wall seconds, user/system
CPU seconds, `max_rss_kib` (Linux KiB) and filesystem I/O operation counts in a
resource JSON file, without external timing packages; tagged JSON records real
rebuild counts, insertion duration, each boundary, and copy duration. These are
local acceptance ceilings, not advertised production performance claims.

Private result/log/resource files remain in a unique home-cache run directory.
No million JSON dump or intermediate archive is written. Database lifetime and
failure fixture disposal belong to Main; the runner never drops a database.
The corrupt-receipt case intentionally ends with a dirty invalid fixture, so it
must not be reused as a subsequent clean acceptance database.
