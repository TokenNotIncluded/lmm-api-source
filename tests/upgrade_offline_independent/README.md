# Independent offline upgrade acceptance

This is the independent synthetic PostgreSQL 18 acceptance for the v3 offline
upgrade, frozen at Core SHA-256
`2d982f5368f984241cc26bcbf60c38ddf56cf243c2400387a811b6e0cd4ef9f8`.
It reuses the r4 PostgreSQL/systemd fixture, but runs its own CLI scenarios and
SQL comparisons. It does not run the author's three complete self-test modes.

The actual run covered missing and incorrect bindings; a real target COMMIT
whose acknowledgement was dropped; Started plus a target receipt; and SIGKILL
of the real controller after PostgreSQL completed `ALLOW_CONNECTIONS true`.
A separate Unix-socket PostgreSQL startup captured an actual ErrorResponse with
SQLSTATE `28000` and an HBA rejection. Recovery closed connections and recorded
Committed. A physical target restore left the separate authority Committed and
recovery reported divergence, with replay and opening refused. The named old
Core unit was stopped and persistently masked; its restart was refused.

The independent SQL oracle compared all 160 existing tables (22 populated),
including synthetic account, password and API credential rows, all 22 original
sequences and 2,001 existing ACL/owner facts. It also checks that no grants
were added to old objects. The only allowed row change is the exact
`core_meta.runtime_contract` catalog fingerprint transition from the v6
predecessor `acbee20015ea1397adfda06232f25c811956823e00b15e574d74794702f418e5`
to `557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d`;
all other fields of that row must remain equal.

Two earlier runs produced fixture-only false alarms. First, `psql`'s connection
error stated that HBA rejected the connection but did not print SQLSTATE; the
final script reads the real PostgreSQL ErrorResponse instead. Second, an
all-table equality oracle incorrectly included the expected runtime-contract
fingerprint change; the final oracle verifies that exact transition. Original
failures and subsequent independent evidence are retained outside the repo.

## Inputs and limits

The launcher verifies all 1,308 source files against the frozen source pins
SHA-256 `20464fa3e5b34566f43f7ca95a2464530d827b2de118475683fd1915f99e470b`,
the binary and its schema-manifest SHA-256
`47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078`,
the predecessor Core SHA-256
`6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31`,
the target catalog reference, and the in-repo fault helper. The source pins and
binary hashes were also checked against the author's build records. There was
**no independent rebuild or independent attestation of compiler provenance**.

The prior actual run used Linux, PostgreSQL 18.6, two private Unix-socket
clusters and one explicitly named systemd user unit. It did not validate v5,
other platforms or PostgreSQL versions, multi-host authority durability,
whole-site funds, final opening or S11. History COMMIT ACK loss was covered by
the author's self-test and was not independently rerun. PostgreSQL uses Unix
sockets and the old Core uses loopback HTTP; this is not OS-enforced network
namespace isolation.

## Run

Use only disposable synthetic data. The output must be a **new directory outside
the source repository**; it can contain synthetic credentials, SQL output,
logs and, after a failure, stopped PGDATA. Do not commit it. No dependencies are
installed and no Native build is started. The host needs Python 3.11+, PostgreSQL
18 tools on `PATH`, a running user systemd manager, and the shared lock
`/tmp/lmm-delegate/heavy-build.lock`. Inspect disk space before running: the
fixture creates and copies PGDATA. The launcher waits for the lock and applies
an aggregate 2 GiB Max / 1.5 GiB High / 4 GiB Swap / one-CPU resource slice.

Set `FREEZE_DIR` to the frozen `compile-r3` directory, `PREVIOUS_CORE` to the
predecessor binary, `REFERENCE_JSON` to the independently generated reference
JSON containing `target_catalog_sha256`, and `EVIDENCE_DIR` to the new private
output path, then run from the source repository:

```sh
python3 -B tests/upgrade_offline_independent/launch.py \
  --core "$FREEZE_DIR/lmm-core" \
  --previous-core "$PREVIOUS_CORE" \
  --source "$FREEZE_DIR/source" \
  --source-pins "$FREEZE_DIR/source-pins.json" \
  --reference "$REFERENCE_JSON" \
  --output "$EVIDENCE_DIR"
```

The launcher writes input binding, commands and result in the evidence
directory. `run/independent-result.json` and the exact ACL comparison are both
required for a successful launcher exit. Successful temporary PGDATA is
removed; failed synthetic PGDATA and logs are retained with clusters stopped.
`fixture.py` is adapted from `scripts/test-upgrade-offline.py` r4, SHA-256
`5ec5eb93691edf5ff12abf6158b1c95ff8d1da5a94704ec51edb01fe39361cf4`;
the target COMMIT fault observer is the existing
`tests/console/independent_process/pg_fault.py`.
