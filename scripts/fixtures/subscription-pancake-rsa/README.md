# Core–Go Pancake RSA subscription fixture

`cached-evidence.json` records the existing private-cache PASS and root's
independent target-state review. **This repository adaptation has not run PG.**
Only static/input checks have run against the adapted entry. Do not transfer the
cached PASS to changed harness inputs or newer binaries.

Run only in an explicitly assigned exclusive PG window:

```sh
python -B scripts/fixtures/subscription-pancake-rsa/prepare.py --check-only
scripts/fixtures/subscription-pancake-rsa/RUN.sh --authorized-pg-window
```

Optional `--core PATH --go PATH --output-parent DIR` arguments follow the window
flag. Binary hashes remain fixed; these options cannot admit a different build.
`pins.json` binds actual Core65 `c0b4cdb8…` (26eb source), actual Go
`476077f1…` (26162 source), both build input manifests and receipts, and reviewed
SQL from Git object `26eb4aebfe63d9ed8181dc7a51c6fd7d831947a8`.
No live working-tree SQL is copied. The Go build uses buildvcs=false; its source is
bound by its own inputs.json/result.json. It includes optional wallet ReturnURL
fix `553982e`; this fixture still uses nonempty URLs. This pair is not the future final
Web/price/payment candidate and cannot qualify that future combination.

Requirements: Linux user/mount/net namespaces, user systemd bus and cgroup
limits, PostgreSQL tools, iproute2, mount/util-linux, Python with cryptography,
and the pinned binaries and Git SQL object. No compiler is invoked. Default
output is a unique private directory under `~/.cache/lmm-subscription-rsa-repo/`.
Inputs are checked before materialization; mount, free capacity, existing size
and available RAM are recorded before PG starts. One parent scope contains the entire PG/Core/Go/Python/synthetic-provider
topology and enforces a combined 1024MiB, zero swap, CPU100%, nice19, idle IO, with one small PG/Core/Go topology.

The fixture preserves the original seven business groups: actual catalog and
runtime authority checks; authenticated TCP prepare via Core UDS RPC;
idempotency and cross-user denial; fixed-origin SDK checkout; RSA-SHA256 wrong
signature and amount rejection; initial entitlement; Go restart and exact
signed replay; renewal and replay with target cycle/grant binding and unchanged
wallet ledger. It also makes one real Core HTTP model call with an HTTP-issued
subscription_only API key and the callback-created current grant. The loopback
Chat supplier reports 7 input + 5 output tokens; fixture rates 2/4 credits per
token must settle 34 (configured rates are 4000000/8000000 micro-USD per
million tokens, converted at the fixed 500000 credits/USD), with used=34, reserved=0 and remaining=966. Actual SQL
charging/command receipts and admin records must agree. All core_billing tables
are hashed before/after; signed callback replay must not reset consumption.
Synthetic user/key routing settings are seeded; Native bootstrap supplies
explicit developer access, and all three are read back before dispatch.
Only routing access is seeded; entitlement/consumption cannot be SQL-inserted.
Configuration, tokens, private keys and database URLs are
generated only in the private output. Real provider HTTP is forbidden.

Private read-only hosts and NSS mounts resolve `api.waffo.ai` solely to
`203.0.113.7` on isolated lo. The namespace must differ from its parent, have
only lo and no IPv4 route or IPv6 default route. Process-local CA files allow
the real SDK's unchanged direct transport to reach only the local TLS stub.
No product Proxy/SSRF guard or host hosts/NSS/CA configuration is modified.
Cached guard-only evidence proved that the pinned production transport accepts
this address, ignores a bad proxy and verifies the private TLS certificate.

One invocation, no automatic retry. Any actual failure is retained and stops
the run; finally stops owned services/PG. `report.json`, commands, HTTP trace,
raw signed replay evidence, target snapshots and host-integrity records remain
private. Check `cleanup` even after failure. The three earlier fixture business
FAIL attempts and the initial NSS guard FAIL remain in the original cache;
their hashes are evidence, not passes. This synthetic run proves neither a
real-provider transaction nor production migration or deployment.
