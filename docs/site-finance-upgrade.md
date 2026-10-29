# Site finance 27 → 28 offline schema upgrade

`lmm-core --config /absolute/installer.toml upgrade-db` reads one JSON request from stdin. The fixed procedure `core-site-finance-v1` accepts only the exact 27-component predecessor and its compiled 28-component successor. It adds the site-finance schema and an owner-only receipt; it does not import historical finance records or open a site. Normal startup does not run this upgrade.

The v1 request has this shape. Values for the database incarnation and roles must describe the actual offline target; never copy these examples into a live operation without the separate reviewed site procedure.

```json
{
  "version": 1,
  "procedure": "core-site-finance-v1",
  "upgrade_id": "example-finance-upgrade-001",
  "database": {
    "name": "coretarget",
    "oid": 12345,
    "system_identifier": "1234567890123456789",
    "server_version_num": 180006
  },
  "roles": {
    "installer": "lmm_core_installer",
    "owner": "lmm_core_owner",
    "runtime": "lmm_core_runtime",
    "extensions": "lmm_extensions"
  },
  "predecessor_manifest_sha256": "47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078",
  "predecessor_catalog_sha256": "557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d",
  "successor_manifest_sha256": "79a672822da5939470cf896a8fab6da7c47e2288f716fe4d505bc3c6e498b858"
}
```

For the intermediate R3/R4 schema, the successor catalog is `ed7dd5a1d9b6f0c620e80c95826d9052bd18573af8f147e3319a1a1126f0af20` with 7,029 definitions. The planned R5 `native_intent_floor` change requires a newly generated, separately pinned manifest and catalog; the R4 catalog must not be used to certify R5. The request binds the successor manifest; the compiled procedure checks the successor catalog and definitions. The installer must be a distinct NOINHERIT login allowed to SET ROLE to the NOLOGIN owner. The procedure checks the actual database identity, exclusive connection state, roles and permissions. It does not itself establish the full persistent service, HBA, external history or site-opening fence.

The development R3 Native with SHA-256 `5144bf9c3d4fc597ae12bd80a0f4e0d5953aa394676a14023d42bc0eb6a35cf6` rejects this procedure before connecting with `UPGRADE_DB_USE_PRESERVED_ROUTING_INSTALLER`. Its shared target authorization applies an older routing DDL pin before dispatching to the finance-specific target check. The synthetic test confirmed the predecessor catalog but did not reach finance DDL or a successful upgrade on R3; a corrected Native and rerun are required.

The intermediate R4 Native `5d27c53627fa038b200040db790599230509c9cdb1067a56064f3ca79e9d374a` subsequently committed the upgrade, preserved the old rows/sequences/ACL, rejected the injected owner default ACL and passed exact replay. Its runtime `check-db` and startup passed; its login test had omitted the required Origin header, and the corrected fixture has not yet been rerun. R4 evidence is not R5 acceptance.

After a confirmed commit, replaying the identical request returns `already_committed`; a different ID or changed binding is rejected. An unknown commit outcome is not proof of failure: preserve the original request and inspect the durable state under the separately reviewed recovery procedure before taking further action. This fixed-DDL selftest does not exercise a lost COMMIT acknowledgement.

## Synthetic developer selftest

`tests/site_statistics/upgrade_selftest.py` uses the actual previous Core `d6d356311de8cf6589c2ec4429603a0d9308ca634b1aed20b78a6c42b7c76c10`, the supplied completed Native freeze, a new socket-only PostgreSQL 18 cluster and synthetic accounts. It verifies the Native manifests and frozen source pins, uses the old Native to initialize 27 components and create the first owner, and applies the old frozen canonical runtime grants. It checks the old catalog, 171 tables and 22 sequences; exercises `upgrade-db`, receipt and replay; compares old rows, sequence states, password rows and old ACL facts; and requires the exact successor catalog to equal both the compiled definition list and a fresh 28-component Native installation.

The selftest also makes a separate old database clone with an owner default ACL granting runtime SELECT on new `core_meta` tables. It records whether the shared preflight or the post-DDL private privilege check rejected it and requires the old database state to survive. After upgrade it explicitly enables only the synthetic runtime role and database CONNECT to check the new Native `check-db`, readiness and old owner password. Changing `finance_native_payments` to `security_barrier=false` keeps its columns but must make `check-db` reject; restoring it must restore validation. This fixture opening is not product finalization.

For a freeze containing `native_intent_floor`, `--check-native-intent-floor` creates a separate post-upgrade database clone, inserts a synthetic source using the original 12 supplied fields, and checks the generated floor against the actual payment sequence high-water. It then changes the clone's sequence increment to two and verifies that the native aggregate marks the result incomplete, restoring increment one afterward. This does not simulate a full historical import or prove the overlapping-payment business cases.

`--check-native-seal-http` creates two further synthetic clones and tests the ID boundary through the actual Native HTTP endpoint. An intent allocated before the seal but credited afterward must return 503 even when its creation time is one day after the dump-start observation. An intent allocated after the seal must aggregate successfully even when its creation timestamp is the same second as that observation. The expected 123-cent gross, zero refund and 615,000-credit balance are fixed by the fixture inputs, not inferred from the response. These are synthetic payment and proof rows, not real provider verification or a historical archive import.

Use an unused private output directory under `Documents/Codex`. Before running, use the `space-first` checks and ensure at least 2 GiB available memory. The command itself takes `/tmp/lmm-delegate/heavy-build.lock`; run it in a scope with `MemoryMax=2G`, `MemoryHigh=1536M`, `MemorySwapMax=4G` and `CPUQuota=100%`, for example:

```sh
systemd-run --user --scope -p MemoryMax=2G -p MemoryHigh=1536M \
  -p MemorySwapMax=4G -p CPUQuota=100% \
  python3 -B tests/site_statistics/upgrade_selftest.py \
  --freeze /absolute/completed/native-freeze --output /absolute/new/private-evidence \
  --target-manifest EXPECTED_MANIFEST_SHA256 --target-catalog EXPECTED_CATALOG_SHA256 \
  --target-definitions EXPECTED_DEFINITION_COUNT --check-native-intent-floor --check-native-seal-http
```

The supplied target values must be the independently fixed values for that freeze; defaults refer to intermediate R4. Success removes this run's disposable PGDATA and socket; failures retain the private data and logs after stopping PostgreSQL. `result.json`, per-command logs and binding data distinguish completed checks from untested cases. The test does not access production or call a real provider. It does not validate full-site fencing, historical finance import, persistent external recovery, COMMIT ACK loss, final opening or production deployment.
