# Existing Core database: offline upgrade v3

`lmm-core --config /absolute/operator.toml upgrade-offline` reads one private
JSON request from stdin. It runs inside the existing Core program. Normal
startup never initializes or upgrades a database.

This procedure covers the target PostgreSQL database and the explicitly named
systemd units. It does not migrate a legacy monolith, certify all financial
rights, inventory a complete site, reopen services, or replace final S11
acceptance. Existing ledger/routing predecessors need their own reviewed
procedure. A successful result leaves the target closed and units masked.

## Fixed inputs

The accepted predecessors are `fresh_v5_core26`, `console_v1_v5_core26` and
`fresh_v6_core27`, pinned in `services/core/src/upgrade_offline/`. Requests cannot
provide SQL or alternative target definitions. v5 has 21 sequences; v6 has 22,
including the agent observation sequence. Exact catalogue fingerprints remain
required for every predecessor.

The successor schema-manifest stdout SHA256 is
`47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078`.
Its PostgreSQL catalogue fingerprint is
`557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d`.
The reference was generated with the previous actual Core installer plus the
exact additive metadata DDL, independently of the upgrade transaction. Fresh
installation by the new actual Core must match it.

`Request` and `BridgeRequest` in `upgrade_offline.rs` and its `engine.rs` define
the bounded JSON format. Set request/bridge version to 3 and bridge procedure
to `core-offline-upgrade-v3`. Bind the actual predecessor manifest/catalogue,
successor manifest, database name/OID/system identifier, distinct installer,
owner, runtime and extensions roles. `candidate_sha256` is the SHA256 of the
actual executable used for apply. `operation_id` identifies this original
operation; changing it cannot clear the site's history.

The request also specifies the history authority ID and private URL file,
management and runtime probe URL files, the canonical HBA path, service names,
scope and executable SHA256, and optional Console/assistant-agent grant deltas.
Base identity email permissions follow the canonical runtime grant script.
URLs and passwords are never request facts in either receipt. The operator
TOML's database URL file selects the dedicated target installer connection.

## Required provisioning

- The history authority follows [upgrade-history.md](upgrade-history.md), in a
  different PostgreSQL instance whose storage, credentials and administration
  are kept outside target restores. Two local clusters only test this boundary.
- Target, management and runtime probe URLs use local Unix sockets. Management
  is a privileged connection to another database in the same target cluster.
- The installer is a restricted LOGIN/NOINHERIT role with a SCRAM verifier,
  explicit CONNECT to the target and SET membership in the NOLOGIN owner.
  NOINHERIT does not supply the owner's CONNECT privilege. The owner must be
  allowed to inspect `pg_control_system()` in the **target database**; runtime
  must not have that function permission there.
- PostgreSQL must already start with `hba_file` pointing at the protected
  regular file outside PGDATA. Restore/start procedures must retain this
  external path. The controller needs permission to replace and fsync it.
  Database administrators and the host administrator remain trusted.
- List each deployment unit explicitly. Active units must match their pinned
  executable. Persistent masks, stopped processes and drained target sessions
  are checked. Missing units and untracked writers cannot be counted as fenced.

## Apply and recovery

Apply reserves external `closing` history, installs a first-match HBA gate,
closes database admission, masks/stops the named units, disables runtime login
and revokes application CONNECT. The HBA admits only the installer over a
local SCRAM connection and rejects other local, IPv4 and IPv6 target clients.
The existing HBA body is retained after these rules.

During maintenance-only connection windows the HBA remains in force. A real
installer connection and a rejected runtime probe check loaded admission rules;
the on-disk `pg_hba_file_rules` view alone is not proof of a loaded policy.
The same dedicated connection takes the installer lock before its transaction.
External history advances through `held` to `started` before target changes.

The serializable target transaction preserves old columns/rows, multiplicity,
sequence state and permissions, installs the fixed additive schema, updates
the contract receipts and writes one immutable local operation receipt.
External `committed` history then binds that receipt. No result grants open or
retry permission. Lost acknowledgement is unconfirmed, never asserted rollback.

For observation after interruption, use the same original binding and change
only `action` to `recover`. Recovery reapplies the admission fence; it is not
read-only with respect to operational controls. It does not repeat schema or
business writes. A bound local receipt plus matching successor contract can
resolve external `started`. External `committed` with a missing/mismatched local
receipt reports `database_diverged_from_committed_history`, including after an
old physical target restore. Missing evidence stays unknown. There is no
automatic finalization, history reset, new generation or site opening.

Normal runtime requires SELECT on the six public contract tables:
`schema_contract`, `component_contract`, `runtime_contract`, `schema_upgrades`,
`component_upgrades` and `ledger_revisions`. Administrative receipts
`console_revisions` and `offline_upgrades` remain owner-only. Runtime startup
rejects table/column SELECT on those private receipts, as well as mutation or
owner authority. Adding an administrative table does not create a runtime
requirement to read it.

## Local verification

`scripts/test-upgrade-offline.py --help` describes the disposable process test.
It uses actual old/new Core executables, independent target/history clusters,
canonical roles/grants, an actual old Core service and real protocol faults.
Use a fresh private output directory and the recorded independent catalogue
reference. `--mode none`, `target-ack` and `history-ack` select normal completion
or dropped actual COMMIT acknowledgements. Each invocation owns its units and
databases; failures remain stopped for diagnosis. These tests are development
evidence, separate from independent business acceptance and production proof.

With `--mode none --verify-runtime`, the fixture explicitly restores access only
to its disposable database, then starts the actual upgraded Core under its
restricted runtime role. It tests existing password login and Notice HTTP
save/read, missing public contract permission, and private receipt table/column
overgrants. The fixture stops that process and restores the gate before testing
physical restore. This manual test opening is not product finalization or
authorization to open a real site.
