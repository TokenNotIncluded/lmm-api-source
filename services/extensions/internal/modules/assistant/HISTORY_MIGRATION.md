# Assistant history v3

History v3 stores user, assistant and human messages in the same conversation domain. Each message keeps its original numeric ID, conversation foreign key, sequence, timestamp and nullable `actor_user_id`. Unknown authors remain null; no operator is inferred. Existing Web history DTOs already accept `human`.

The installer accepts original legacy title/preview lengths and empty strings. Restriction reasons stay private in storage. `legacy_list_hidden` marks imported message-less shells: they stay stored and owner-readable, but remain absent from lists until actual messages exist. Archive state is not invented.

## Explicit offline upgrade

Fresh installs use v3. Existing v2 databases must be fenced and upgraded by the installation login through the NOLOGIN database owner. Runtime stays NOLOGIN until the operation is verified; ordinary startup only checks the schema contract.

```sh
lmm-extensions upgrade-assistant-history-db \
  --owner-url-file "$OWNER_URL_FILE" --database "$ASSISTANT_DATABASE" \
  --owner-role "$OWNER_ROLE" --runtime-role "$RUNTIME_ROLE" \
  --operation-id "$UPGRADE_OPERATION_ID"
```

Use the same command and original ID with `--recover` after an unknown outcome. A different operation cannot impersonate the original upgrade receipt. Runtime receives no access to owner receipts or raw staging tables.

## Reviewed frozen-source projection

After upgrade, a separate owner operation projects exactly one reviewed `owner_only_raw` batch whose lineage is `FROZEN_OFFLINE_LEGACY`. It verifies batch counts, original user IDs and message linkage, refuses a nonempty formal history, and commits the projection and receipt together. Live-only/unknown-lineage corpora are not unioned or made visible.

```sh
lmm-extensions upgrade-assistant-history-db \
  --owner-url-file "$OWNER_URL_FILE" --database "$ASSISTANT_DATABASE" \
  --owner-role "$OWNER_ROLE" --runtime-role "$RUNTIME_ROLE" \
  --operation-id "$PROJECTION_OPERATION_ID" --project-batch "$FROZEN_BATCH"
```

The original raw stage remains owner-only. This operation copies conversations/messages; it does not assert that the separate platform-support appointment workflow has been migrated. No Core client, chat/model call, funding, callback or service start is involved. A developer's isolated PG test is not production or independent user acceptance.
