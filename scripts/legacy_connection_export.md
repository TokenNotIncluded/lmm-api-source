# Frozen legacy connection export v1

Run `legacy_connection_export.sql` against the coordinated, local frozen source
with a read-only connection. No target connection is needed. The transaction
reads every retained row, including soft-deleted tokens and revoked grants.
No assumption about unobservable hard deletion is part of this contract.

```sh
umask 077
PGOPTIONS='-c default_transaction_read_only=on -c search_path=YOUR_LEGACY_SCHEMA' \
  psql -X -qAt -v ON_ERROR_STOP=1 -f scripts/legacy_connection_export.sql \
  > /YOUR/PRIVATE/PATH/connections.source.json
sha256sum /YOUR/PRIVATE/PATH/connections.source.json
```

Set the connection's local socket/database/user independently, as for the other
frozen exports. Do not put credentials in shell arguments. Do not use a production
or DMIT source. The SQL does not connect to or alter any other PG.

The output is exactly one JSON object:

| Field | Shape |
| --- | --- |
| `contract` | `legacy.connection-source.v1` |
| `source_schema` | actual current schema |
| `observed_at` | export transaction time, integer Unix seconds |
| `coverage` | `all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked` |
| `counts` | exact `users`, `tokens`, `oauth_server_grants`, `developer_access_requests` row counts |
| `users` | array of `{id: integer, created_at: integer or null}` |
| `tokens` | array of `{id: integer, user_id: integer, oauth_managed: boolean or null, created_time: integer or null, deleted_at: original timestamp or null}` |
| `oauth_server_grants` | array of `{id: original grant ID, user_id: integer, created_at_ms: integer or null, revoked_at_ms: integer or null}` |
| `developer_access_requests` | array of `{id: integer, user_id: integer, created_at: integer or null, reviewed_at: integer or null, status: string}` |

Preserve invalid/missing timestamps and classification fields as-is; do not turn
them into zero or silently omit rows. They remain auditable UNKNOWN facts.
The importer will derive per-user minima and coverage from the actual rows.

Provide this separate private metadata JSON (do not edit the source JSON):

```json
{
  "contract": "legacy.connection-export-binding.v1",
  "source_archive_sha256": "64 lowercase hex of the ORIGINAL frozen source archive",
  "source_snapshot_id": "existing frozen restore/snapshot identifier",
  "source_schema": "same as output",
  "snapshot_observed_at": 1700000000,
  "export_file_sha256": "sha256sum of exact connections.source.json bytes",
  "coverage": "all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked"
}
```

`snapshot_observed_at` identifies the source freeze, not today's export time. Reuse
the established freeze timestamp from that archive/restore manifest. Explain an
unavailable freeze timestamp instead of inventing one. The source archive hash
must be the one bound by the existing opening and identity migration. The export
file hash is a separate artifact hash; neither substitutes for the other.

For target planning, also reuse the reviewed source-user → target-user/personal-
account mapping, original opening migration ID, and exact original opening entity
keys (`user:N` or `legacy.users/N`). No extra source secrets or monetary export
are required. A target-instance binding will be captured read-only from the NEW
Core55 target after its existing identity/opening/key imports, never from Core54.

The expected 133/1239 grant/token facts are checked against `counts` and array
lengths; all rows remain in the receipt even when their owner cannot be mapped.
A missing owner or invalid timestamp is recorded, not asserted to be a negative.
