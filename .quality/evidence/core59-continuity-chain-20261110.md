# Core59 synthetic continuity migration chain — 2026-11-10

Developer self-test: PASS, one complete-chain case in 40.482 seconds. Implementation and harness commit: `e77d463`. This is not independent acceptance, a production migration, application activation or final-release evidence.

## Fixed scope

The source is a locally created physical legacy-shaped PostgreSQL database, dumped and restored before actual reader checks and exports. The target was initialized by the fixed Core59 executable SHA-256 `07930fa8cdcfb98a646fb52d7bc92a27f13a135e65c8dda1eb0eabf2fcf4b842`, with reviewed schema revision `3579ac7`. Producer/consumer sources were frozen from `93d39c5` plus the harness and timezone fix; the current live Core tree was not used. Root verified all three changed files and the eight named producer/consumer scripts against the successful frozen inputs before committing.

Entry: `Schema58PostgresTests.test_full_physical_legacy_continuity_cli_chain` in `scripts/test_legacy_schema_profile_pg.py`. The historical class name does not identify the tested schema; the case explicitly checks the Core59 contract, compiled manifest and catalog. It does not call the earlier payload-only case.

## Failure and fix

The first actual run passed identity import and stopped before financial writes: financial export compared a preflight server-start timestamp rendered in +08 with the same instant rendered in UTC. The production exporter now sets UTC before preflight identity capture, matching its existing export transaction. No identity fields, strict dictionary comparison or restart checks were removed. Four focused transport-substituted tests cover equal instants with different offsets, a different receipt instant, a restart and changes to other identity fields.

The first full-chain failure remains in `~/.cache/lmm-importer59-full-chain-nv5dimot/attempts/attempt-ff7ifc9q/`. Successful real-PG rerun evidence is `~/.cache/lmm-importer59-reader-timezone-b2bm9047/attempts/attempt-yxxdomys/` (`report.md`, `test.log`, `evidence-sha256.json`). The result is also retained at `~/.cache/lmm-import-profile-pg-i354_yu3/result.json`.

## Proven by the successful run

- Physical dump/restore, actual catalog/count collection and restricted-reader checks precede the real exporter and importer calls.
- Identity CLI apply and replay retain mapped users, dormant privileged identity and expected authority.
- Financial CLI prepare/apply/replay/resolve retain the source-bound opening balances; rejected approval leaves target state unchanged.
- Subscription preparation generates the real reviewed apply/resolve SQL, executed against PG. Imported consumption, finite/unlimited and current/future period limits are read back.
- Routing CLI preparation/resolution and its generated SQL preserve original active/inactive group permissions.
- Credential CLI prepare/apply/replay/verify preserves finite and unlimited quota semantics, original key hashes and recoverable ciphertext. The original recoverable key verifies; a wrong recovery key is rejected.
- Every phase checks permitted target changes and same-phase replay/resolution. Replaying upstream phases after later writes is rejected without mutation. Five import receipts bind one archive; final identity/money state, schema catalog, archive hash and source hashes remain intact.

The private PG process stopped and successful PGDATA was removed; failure data and original evidence remain. No application/runtime was started, no old live application behavior was observed, and no production approval is implied. Real source/provider mappings, newer schema profiles, actual login/revocation/charging, final fixed binaries and independent production acceptance remain outstanding.
