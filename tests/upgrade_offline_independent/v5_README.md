# Independent v5 predecessor acceptance

These scripts exercise the real `upgrade-offline` CLI on two synthetic PG18
predecessors, not the author's internal rollback-only stage test. They use the
r4 historical component SQL as read-only input, checking every component hash,
installer contract, manifest bytes, exact catalog and table/sequence counts.
The `console_v1` fixture adds a synthetic immutable v1 receipt; it does not
claim to have run a historical Core executable. No known v5 Core executable is
used, so requests explicitly have `services=[]` and cover database fencing and
external history, not shutdown of a real old Core service.

The independent oracle checks old rows using the old column lists, all 21 old
sequence values and definitions, old ACL facts, and only the expected explicit
new grants on existing objects. It checks old email aliases and password hashes
remain unchanged, no verified email/reset authority is invented, both CLI apply
and recover observe a real committed receipt, and `site_open_allowed` remains
false. Its runtime phase performs an explicitly fixture-only reopening and
canonical runtime grants, starts the actual Core, and tests alias/password
login and unverified email reset over loopback HTTP. This manual fixture setup
is not product finalization or permission to open a site.

The 2d982f build passed the database portion for both predecessors. Its actual
runtime startup reported `MissingPrivileges`: startup incorrectly requires
SELECT on owner-only `core_meta.offline_upgrades`, while the canonical grant
script correctly does not give runtime that access. The repair candidate has
Core SHA-256 `dea60909b00bed72db09772200db99233b9eb68bdb9a62197aa38288786d71aa`
and source-pins SHA-256 `ac32f15464d42b4cf7f164e510af79d9e3183dc5fb56ec68cf2c135bf298f9e8`;
only `runtime_database.rs` differs from the 2d982f input set. Both v5 cases
passed actual CLI apply/recover and the explicitly fixture-opened Core runtime
checks on that candidate. No fixture grant is added to bypass the check: the
negative controls temporarily grant table-level and column-level SELECT on each
private receipt solely to verify `UnsafePrivileges`, then revoke it. Final opening, full site funds and S11
remain unverified. The first old-sequence
comparison also had a fixture error and was corrected to use the exact 21 names
captured before the upgrade; that attempt is not counted as a product failure.

`v5_launch.py` pins the actual Core binary, the 1,308-file source pins manifest,
and the PostgreSQL fault helper for both the r3 and runtime-fix profiles. It
uses the same shared lock and bounded aggregate slice.
An independent rebuild or independent compiler-provenance attestation was not
performed. The two profiles must not be conflated as evidence of one binary.

Set `FREEZE_DIR`, `HISTORICAL_SQL`, and a new private `EVIDENCE_DIR` outside the
repo; inspect space first. With PostgreSQL 18, systemd user service and Python
with the already-installed `argon2` module available, the database-only phase is:

```sh
python3 -B tests/upgrade_offline_independent/v5_launch.py \
  --core "$FREEZE_DIR/lmm-core" \
  --source "$FREEZE_DIR/source" \
  --source-pins "$FREEZE_DIR/source-pins.json" \
  --historical "$HISTORICAL_SQL" \
  --output "$EVIDENCE_DIR" --database-only
```

For the separately frozen repair candidate, point `FREEZE_DIR` at its source
and binary, replace `--database-only` with `--freeze-profile runtime-fix`, and
inspect the resulting runtime evidence before declaring acceptance. Runtime
checks include old-alias login and password, rejection of wrong/disabled/
ambiguous aliases and unverified resets, a separate fixture-verified positive
reset control, and refusal when a required public contract grant is removed.
The output is private evidence and may contain synthetic secrets and logs;
never commit it. On failure, stopped PGDATA is retained. A database-only zero
exit is explicitly recorded as `DATABASE_PASS_RUNTIME_PENDING`, not full
acceptance. `v5_diagnose.py` reproduces the startup privilege failure on a
private copy of the stopped synthetic PGDATA; the original is not modified.
