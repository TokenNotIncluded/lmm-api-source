# Synthetic PG18 channel-opening owner acceptance

This harness is authored but **PG execution is NOT RUN** until Main supplies and
authorizes the isolated fixture. Pure tests do not establish generated SQL
execution. No Cargo, downloads, cluster initialization, schema patching,
production input, or original template DML occurs here.

Default is a plan without connections or subprocesses:

```sh
python3 -B tests/channel_openings/importer_selftest.py
```

Main's later execution interface:

```sh
python3 -B tests/channel_openings/importer_selftest.py --execute \
  --connection /absolute/private/evidence/synthetic-admin.json \
  --core /cache/cargo-target/debug/lmm-core \
  --output /absolute/private/evidence/new-empty-owner-pg-run
```

Use the actual absolute binary path. Main must run space-first before approving
the database clones; this invocation may cumulatively exceed 100 MB. The output
must already exist, be empty and mode 0700. Referenced connection/pgpass files
must satisfy the existing owner tool's private 私有材料目录 path checks. This harness
never scans 私有材料目录 for inputs. Its only directory listing checks the explicitly
provided output for emptiness.

The connection file uses the existing local socket fields `socket_directory`,
integer `port`, `database`, direct administrative LOGIN `role`, `pgpass_file`,
plus Main's `synthetic_only: true`. `database` names the **existing synthetic
Core44 template**. The resource owner must be able to create only the disposable
test resources on this authorized cluster; do not point it at production.
The supplied server must be PG18, Unix-only, and the template's component
fingerprints must match `lmm-core schema-manifest` from the precompiled binary.
This is a template adapter, not another init-db implementation.

Dependencies that deliberately fail instead of weakening acceptance:

- The template must have **nonempty canonical synthetic charging requests** and
  a ready/clean channel projection. Provision those with the existing charging
  fixture owned by Main, not hand-written placeholder requests in this harness.
  It must have no legacy opening source. The three fixture IDs 880001–880003
  must be unused. Template clients must be disconnected for CREATE DATABASE
  TEMPLATE. No HTTP/model request is sent.
- The normal installation NOLOGIN owner must retain the existing owner-import
  read privileges, including the actual control-system/sequence/catalog reads.
  `--runtime-role` defaults to `lmm_core_runtime`; a new fixture NOLOGIN role
  inherits its effective privileges for the existing siteFinance owner check.
  Existing roles, passwords, LOGIN flags, ACLs and server settings are not reset.
- `/usr/bin/psql`, `pg_dump`, `pg_restore` must already exist. Source export uses
  actual pg_dump → pg_restore into a different database, independently compared
  source/restored rows, and a restricted READ ONLY channel/options export.

The general financial exporter already classifies `channels` but also requires
users/tokens/top-ups/subscriptions. A channel-only synthetic source cannot call
that exporter unchanged. The narrow test adapter reuses its identity/schema/row
framing, metadata files and `read_frozen_export`; it does not mark an arbitrary
JSON file as a real archive. Archive/export/proof/mapping/evidence hashes come
from actual generated files, with the existing 1 MiB streaming hash helper.
The test source explicitly constructs all counters with K=3, archives the source
option, and binds that declared synthetic unit history to archive/export/rows.
This does not establish K for any real historical source.

Groups cover actual CLI preview/apply, persisted 7/3 and signed -1/3 offsets,
included combined Native baseline subtraction (7/3 − 150000/500000 = 61/30),
exact global opaque request IDs, original balance epoch preservation/zero→null,
fresh provider preservation, lost successful acknowledgement→UNKNOWN→read-only
resolve COMMITTED, target drift→refusal→resolve UNKNOWN, UNKNOWN basis/overlap
without SQL dispatch, original-material NOOP preview and execution of generated
NOOP SQL. Independent siteFinance full-table/sequence/catalog/ACL capture allows
only the explicitly expected channel display mutations; all ledger/financial
source tables remain unchanged. The supplied template is compared before/after.

The post-seal Native change is an explicit **display counter-baseline metadata
delta**. It proves the owner workflow does not recalculate the old seal/offset;
it is **not actual settlement/refund or Native ID-growth acceptance**. Those
remain the canonical charging/projection suite's responsibility. No fixture
money source, funding, journal or charging receipt is manufactured by this test.

Each run creates UUID-prefixed databases and roles and records their exact names
in private `owned-resources.json` / `result.json`. They are retained for Main's
inspection and scheduled cleanup, including on failure; the harness never stops
or reconfigures Main's server. Detailed command stdin/stdout/stderr stay private.
Only `--execute` may issue SQL; writing and static-checking this harness does not
authorize or report any group as executed.
