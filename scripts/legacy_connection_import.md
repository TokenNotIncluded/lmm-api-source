# Core55 retained connection snapshot import

This is a fresh-target, owner-only analytics import. It never creates live keys,
OAuth grants, login bindings, money, permissions or developer approval authority.
No existing-site upgrade or real-source verification is implied by synthetic tests.

`legacy_connection_export.sql` exports all retained users, tokens, grants and
application rows, including soft deletes/revocations, in one read-only transaction.
See `legacy_connection_export.md` for the exact source fields and metadata JSON.
The declared coverage is the retained frozen snapshot; proving the absence of
hard-deleted history is outside this contract and is not required.

Preserve `source.json` bytes and their SHA256 separately from the original source
archive SHA256. Supply metadata unchanged, including source snapshot identifier,
schema and original freeze time. Missing times/classification remain raw UNKNOWN.
Source owners not present in the source user roster stay in the raw receipt as
unresolved rows. No source secret is an input.

Mapping JSON is explicit, including omissions:

```json
{
  "contract": "legacy.connection-mapping.v1",
  "users": [
    {"source_user_id": 101, "user_id": 11, "account_id": 1,
     "opening_entity_key": "legacy.users/101"}
  ],
  "omitted_users": [{"source_user_id": 102, "reason": "reviewed mapping not available"}]
}
```

The two arrays must partition the entire retained source user roster. No duplicate
source or target users. Omitted target users are not treated as negative evidence.
For every mapped user Core checks original opening migration/entity/account,
original archive, opening mapping digest, and the source registration time in the
original `legacy-cutover-identity.v1` receipt. An identity receipt without these
per-account registration bindings is unsupported; it must not be forged to pass.

On a **new, stopped Core55 target**, complete the existing reviewed identity,
opening and credential imports first. This reader supports the current cutover
credential importer that preserves token IDs. Each existing API-key credential
must match its original retained source token's ID, creation time and mapped
personal owner. A native key created before this history import is rejected rather
than guessed to be a legacy row. No other credential mapping format is inferred.
Later native keys (above the sealed credential boundary) and native browser/market
OAuth families are combined with the historical minima.

Run `legacy_connection_target.sql` read-only on that new target to obtain one JSON
object with `contract=legacy.connection-target.v1`, database name/OID, original
instance nonce, installed catalog SHA256, explicit `component_count=55` and current
credential boundary. The instance nonce must be the target's existing nonzero
64-hex database-level migration setting, never a session override. No old target
receipt or different target may be substituted.

Generate a new private SQL file (the command does not connect anywhere):

```sh
python3 scripts/legacy_connection_import.py \
  --source /PRIVATE/source.json --metadata /PRIVATE/metadata.json \
  --mapping /PRIVATE/mapping.json --target /PRIVATE/target.json \
  --opening-migration-id ORIGINAL_OPENING_ID \
  --opening-mapping-sha256 ORIGINAL_OPENING_MAP_SHA256 \
  --identity-migration-id ORIGINAL_REGISTRATION_RECEIPT_ID \
  --output /PRIVATE/new-owner-only-plan.sql
```

Output creation is exclusive and mode 0600. Execute the reviewed plan explicitly
as the installation owner against the bound target. The plan locks relevant
identity/provenance tables and inserts one immutable snapshot in a serializable,
durable transaction. Exact replay, changed target/nonce, wrong archive/opening,
wrong personal owner, changed credential boundary and duplicate import all reject.
Runtime has no INSERT/UPDATE/DELETE/TRUNCATE authority on this table.

The source's exact UTF8 bytes, file hash, mapping bytes/hash and binding metadata
are retained in `core_history.connection_snapshot`. No regenerated source JSON
substitutes for the input hash. The private community reader emits only requested
user milestone times, per-stage coverage and `history_source`; no source row ID,
credential ID or document leaves Core. Batch limit is 100 users.

Historical `oauth_authorized` is the earlier of OAuth-managed token creation and
grant creation (milliseconds floored to seconds); ordinary token creation is the
API-key milestone. Application/approval minima use retained access requests.
Neither login bindings nor current active state is a substitute. Known absence
within a fully bound retained snapshot is usable evidence; genuinely missing or
invalid times/classification remain UNKNOWN. Timing is exact only with coverage.

Native self-tests, synthetic historical import/readback, real-source verification,
independent acceptance and deployment are separate outcomes. In particular,
synthetic retained counts do not prove the future real 133/1239 source facts.
