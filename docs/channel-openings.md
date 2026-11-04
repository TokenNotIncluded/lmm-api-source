# Legacy channel openings

`scripts/legacy_channel_accounting.py` imports reviewed channel **display facts**
into the owner-installed component 44 tables in
`services/core/schema/channel_openings.sql`. It never funds a wallet, creates a
charging receipt, changes a Native counter baseline, publishes routing, installs
schema, or contacts a source database. Default mode is a file-only preview;
`snapshot`, `apply`, and `resolve` are explicit operations.

The historical implementation and its QuantumNous/new-api and contributor
attribution remain under AGPL-3.0-or-later. This procedure does not replace the
original archive or reinterpret its counter as a complete ledger.

## Meaning and precision

The legacy `used_quota` is a **recorded signed int64 counter**. The historical
implementation adds positive and negative deltas, including some refunds, and
updates some counters before settlement completes. It is neither a proved gross
amount nor automatically a complete settled net amount. Preserve the exact raw
integer, its proved historical K (quota units per USD), and the original source
observation time. Do not derive K from a current startup setting or infer total
channel consumption by summing available logs. Per-log captured pricing is not
proof of a whole counter's unit history.

Historical counter USD is the exact rational `raw_used_quota / K`. K need not
equal 500000 or divide Native credits evenly: raw 7 with K=3 stays 7/3 USD; it
does not become a fractional u128 credit balance. The installed DDL admits a
positive safe-integer K and signed NUMERIC(78,0) numerator/denominator. A known K
outside that domain produces preview UNKNOWN and requires a schema decision;
the script does not round it. Finite cached provider balance is separate and
retains its provider unit; it is never divided by K or a wallet exchange rate.
The destination balance column remains float8, with its existing round-trip
precision, not an invented exact provider ledger.

Two overlap declarations are supported, each requiring archive-bound evidence:

| Declaration | Imported offset USD |
| --- | --- |
| `disjoint` | recorded legacy raw / proved historical K |
| `included_at_seal` | recorded legacy raw / K minus sealed Native combined net / 500000 |
| `unknown` | preview UNKNOWN; no apply SQL |

Combined Native net includes own projected settled gross minus refunds **and**
the existing copied Native display baseline's gross minus refunds. Subtraction
happens once at the seal. Later settlements and refunds change Native totals
normally; they do not recompute the legacy offset. The seal stores every exact
opaque `core_charging.requests.id`, including IDs outside the mapped channels;
neither timestamps nor numeric/max ID guesses perform deduplication.

For included mode, mapped pending requests, mapped unknown attribution, or global
unknown attribution block confirmation. An unknown copied Native baseline with
a non-null source channel blocks both modes. A root baseline whose source is
null may receive an evidenced legacy opening. Projection readiness, empty dirty
queue, and contribution/total-rollup consistency are required at first apply.
An included negative offset needs separate reviewed correction evidence; a
negative disjoint recorded counter remains signed without clamping. A claim of
complete historical coverage also needs explicit evidence. Without that claim,
the recorded subtotal is still not a lifetime/net completeness guarantee.

Only an **existing** `unknown`/null balance observation can receive a source
cached value. It becomes `copied`, has null external `source_channel_id` and
credentials, and preserves the original provider observation time. Original
time 0 becomes null, never restore/import time. Provider, copied, reset, and
missing observations are preserved. `recorded_at` is the new import recording
time; it is not presented as the provider's observation time.

## Private input contracts

All artifacts and referenced files must be owned, private, non-symlink files
under `/home/lightjunction/Backups`; directories are 0700 and files 0600. Output
directories must already exist and be unused. Writes are O_EXCL, fsynced, and
never overwrite evidence. Do not put raw exports, secrets, database connections,
approvals, or materials in Git. Nothing below authorizes production access.
Only explicitly supplied local synthetic/frozen files and their receipt-listed
metadata are read; Backups is never scanned for inputs. Archive and referenced
file hashes reuse the existing 1 MiB streaming SHA-256 helper. Proof files are
checked for nonempty size without loading their entire contents.

The file manifest maps names to `{ "path": "absolute private path", "sha256":
"file SHA-256" }`. Required source entries are `archive`, `source_receipt`,
`source_export`, `source_export_proof`. Preview additionally requires `mapping`
and `unit_evidence`. Separate nonempty proof files use names prefixed
`evidence:`. File SHA-256 means a hash of actual bytes; row/material digests use
the existing financial planner's type-tagged `F.digest`, never a hand-written
JSON serialization or SQL hash.

Reuse the existing frozen snapshot receipt and financial export/proof tools.
The receipt must be `snapshot-restored-baseline-verified-frozen-and-classified`.
The export must include the complete original `channels` table columns/rows;
metadata hashes, selected row counts, source instance framing, and archive/export
hashes are checked with the existing financial reader. An existing wallet-only
export is insufficient. No automatic export, restore, or DB connection occurs.
The script rechecks the receipt's private metadata files against the export
proof each time it validates the material.

Mapping JSON has exactly these fields:

```json
{
  "format_version": 1,
  "archive_sha256": "<archive file hash>",
  "export_sha256": "<export file hash>",
  "source_revision": "<exact historical source SHA>",
  "observed_at": null,
  "channels": [{
    "source_channel_id": 17,
    "channel_id": 1,
    "source_row_sha256": "<full original row type-tagged digest>",
    "historical_coverage": false
  }]
}
```

`observed_at` is the original archive/counter observation epoch seconds, nullable
when unknown; restore time is not a substitute. Source and target IDs are unique
positive safe integers, explicitly mapped to current unambiguous active target
identities. Full source-row hashes bind even fields omitted from the display
material, including private keys, without copying those keys into the material.

Unit evidence has this shape (angle-bracket values are placeholders):

```json
{
  "contract": "legacy-channel-opening-v1",
  "archive_sha256": "<archive file hash>",
  "export_sha256": "<export file hash>",
  "mapping_sha256": "<mapping file hash>",
  "rows_sha256": "<inspection mapped rows digest>",
  "native_overlap": "disjoint",
  "overlap_proof_sha256": "<referenced overlap proof file hash>",
  "channels": [{
    "source_channel_id": 17,
    "source_row_sha256": "<full original row digest>",
    "credits_per_usd": "3",
    "basis_kind": "captured_counter_unit",
    "proof_sha256": "<referenced counter denomination proof file hash>",
    "balance_unit_label": null
  }]
}
```

`basis_kind` is `captured_counter_unit` or `audited_counter_unit_history`.
`startup_current` is not evidence. Unknown K uses null; unknown overlap uses
`unknown`. An evidence file's presence and hash do not establish its truth: the
owner must review its archive/export/row binding and substantive historical
denomination/overlap proof before approving. Where requested, per-channel
`coverage_proof_sha256` must reference evidence that the historical counter
coverage and semantics justify the completeness claim; per-channel
`negative_offset_proof_sha256` must justify an otherwise anomalous included
negative offset. These are explicit reviewed source assertions, not guesses
from available logs or current settings.

## Offline workflow

Use `python3 -B scripts/legacy_channel_accounting.py --help` for arguments.
Examples below only name **prepared local owner files**; do not execute them
against production or use them to initialize a database. Choose existing unused
private output directories under Backups for every step.

1. Run `inspect --files <source-manifest> --output <inspection-dir>`. This is
   file-only and needs only the four frozen-source entries. Its private output
   gives source IDs, exact raw counters, balances/times, and original full-row
   digests, without keys. Author the explicit mapping, add its file reference,
   then inspect again into a new directory to obtain `rows_sha256`. Author the
   historical unit/overlap evidence, add its reference and proof-file references.
2. At Main's explicitly scheduled offline window, stop runtime clients and fence
   other LOGIN roles' database CONNECT. Use the existing NOLOGIN installation
   owner through its separate restricted LOGIN operator/SET ROLE pair. Connect
   only through its private local Unix socket; TCP and nonempty listen_addresses
   are refused. Run `snapshot --connection <private-connection>
   --runtime-role <runtime-role> --output <snapshot-dir>`; repeat `--runtime-role`
   for all runtime roles. This operation is read-only. It hashes all application
   rows and sequence state, catalog, owners/ACLs, instance/system identity, exact
   request IDs, projection, channel identities, baselines, and balances.
3. Run `preview --files <manifest> --target <snapshot-dir>/target.private.json
   --runtime-role <runtime-role> --output <review-dir>`. This is file-only. READY
   emits original `material.private.json`, deterministic `review.private.sql`,
   and `approval-request.private.json` with decision `REVIEW_REQUIRED`. UNKNOWN
   emits material/reasons but no apply SQL or approval request. Never edit
   derived rows, offset, seal, or plan to make an UNKNOWN preview READY.
4. Review the source assertions, signed raw/K, exact rational offsets, balance
   eligibility, Native request seal, target hashes, roles, and SQL. Only after
   review, create a separate private approval JSON with precisely the requested
   hashes and decision `approve-offline-channel-opening`; do not edit material.
   Create `<review-dir>/apply` with mode 0700. Run `apply --material
   <review-dir>/material.private.json --approval <approved-json> --connection
   <private-connection> --output <review-dir>/apply` **once**. The material binds
   this fixed attempt directory, preventing resubmission with the same material
   merely by choosing a fresh output directory.
5. On UNKNOWN, keep all evidence and never rerun apply. Run `resolve --material
   <original-material> --approval <original-approval> --connection
   <same-private-connection> --attempt <review-dir>/apply/attempt.private.json
   --output <unused-resolution-dir>`. Resolve checks the original persisted
   attempt, the shared owner advisory fence and absence of the original live
   session, then uses a read-only transaction. A matching immutable source
   receipt and original imported rows establishes COMMITTED. Absent receipt
   remains UNKNOWN, including after a restored backup. Resolve never submits
   INSERT/UPDATE or infers rollback; Main/owner must adjudicate unresolved state.

The connection JSON reuses the existing financial importer contract:
`socket_directory`, integer `port`, `database`, private `pgpass_file`, NOLOGIN
owner `role`, distinct `login_role`, and `set_role` equal to `role`. The target
instance comparison also binds session role, database OID, system identifier,
postmaster start, server nonce, and catalog/ownership. A restart or tool/schema
change invalidates the prepared identity/approval and requires owner review;
never use a new material as an automatic retry of an uncertain attempt.

## Receipt and NOOP rules

`opening_sources.plan_sha256` is the owner receipt. No extra receipt table is
created. It binds the original material's source, exact global request seal,
target snapshot, derived imported rows and balance patches, and tool inputs.
`source_key` binds immutable archive/export/mapping/unit-evidence hashes, source
revision/time, overlap declaration and mapped source rows. It deliberately
excludes the mutable target Native seal. Single source namespace is enforced by
the installed table index.

First apply takes the shared financial/identity advisory lock, locks application
tables, requires the exact original target snapshot, inserts typed rows via
`jsonb_populate_recordset`, copies only eligible balances, and verifies readback
and unchanged unrelated table/sequence state. The public success receipt is
selected **after COMMIT**. Transport failure, ambiguous output, post-submission
input changes, or failure to save the result stays UNKNOWN; no automatic retry.

For a later file-only reconciliation, take a new explicitly authorized read-only
snapshot and run `preview --material <original-material> --target
<new-snapshot> --output <unused-noop-dir>`. A matching source receipt and original
imported rows produces NOOP. Native IDs, settlements, refunds, pending counts,
new channel copies and provider observations can grow normally. They do not
replace sealed IDs or recalculate offsets. Copied opening rows are allowed;
original imported rows and their archive/export/mapping/unit evidence/material
identity must still match. Rebuilding a fresh opening from an occupied namespace
produces UNKNOWN and instructs use of the original material. NOOP does no DML.

Runtime copy false preserves the exact legacy offset and the separate Native
baseline snapshot; later source refunds do not modify the copy. Reset true does
not inherit the opening. These Rust operations are owned by Main; this owner
script neither implements nor invokes them.

## Local verification

Run `python3 -B scripts/test_legacy_channel_accounting.py`. The synthetic tests
exercise exact signed/rational plans, unknown evidence, included-mode guards,
sealed NOOP after Native growth/refunds/copies, constrained generated SQL, and
single-attempt/resolve transport orchestration. They use no PostgreSQL/network.
They do **not** establish SQL execution, runtime permission acceptance, actual
transaction rollback/commit, full builds, browser acceptance or production
migration. PostgreSQL/owner-role acceptance must be scheduled separately by Main.


## Fresh cutover channel identities

The fresh cutover runner registers `channel_identities` immediately before
`channels_history`. Run it before starting Core with model configuration or
publishing routes. The target must have no channel identities, routing versions,
active routing release, channel openings or Native charging requests. The empty
routing singleton created by `init-db` is allowed.

Supply `--channel-identity-mapping /absolute/private/channel-identities.json`:

```json
{
  "format_version": 1,
  "archive_sha256": "<original archive SHA-256>",
  "observed_at": 1793674367,
  "source_schema": "lmm_prod_20260802",
  "channels": [{
    "source_channel_id": 17,
    "channel_id": 17,
    "source_row_sha256": "<legacy_cutover_history.sha(full source row)>"
  }]
}
```

Include every source channel, including disabled channels, exactly once, keeping
its original ID. Use the same archive, observation and `--migration-id` as the
opening import. This fresh protocol uses `legacy_cutover_history.sha` for full
row digests; the standalone opening file package above uses `F.digest`.

The installation owner applies one SERIALIZABLE transaction. It checks table
ownership, the matching opening run and the empty target, then records the full
mapping, original status and source row hashes in
`core_migration.import_runs`. It creates permanent `migrated` identities,
`historical_coverage=false` counter baselines and `unknown` balance observations.
The receipt contains no original channel key. Existing identities, including
bootstrap, retired or ambiguous IDs, are rejected. Run the separate
`channels_history` phase next to import supported counter offsets and cached
balance observations. Route publication and credential configuration remain
separate operations.

In the local fresh component-51 rehearsal, all 29 original identities and the
following history phase were imported. Five post-rebase counters had proved
units; 24 earlier counters retained unknown units. The four Core ledger tables
had identical SHA-256 aggregates before and after these phases. This evidence
covers local import behavior; it does not establish model traffic, page
acceptance or production readiness.

Independent local acceptance confirmed all 29 source mappings and receipt
rows, five supported offsets, 24 unknown offsets, seven copied observations
and 22 unknown observations. Existing-identity replay and a LOGIN installer
running without the owner role were rejected without state changes. The unified
24-check financial reconciliation still passed, with the channel-identity gap
reduced to zero; other financial and feature gaps remain. Concurrent writers,
wrong-opening and active-routing guard variants were not exercised on real
PostgreSQL.

The targeted planner check is
`python3 -B -m unittest discover -s scripts -p test_legacy_cutover_channel_identities.py`.
