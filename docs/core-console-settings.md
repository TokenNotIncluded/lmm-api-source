# Console public content settings

Core exposes the effective settings through `GET /api/option/`. Its
`capabilities.writable_keys` is the write contract: currently `Notice`,
`Announcements` and `console_setting.announcements`. The two announcement keys
refer to the same content. `partial=true` remains explicit; host TOML, credentials,
financial policy and unsupported settings are not writable through this API.

`PUT /api/option/` accepts one string value. `POST /api/option/bulk` accepts a
`values` string map and optional `expected_values` for compare-and-save;
`POST /api/option/validate` checks the same input without saving. Announcement
content is validated as structured JSON before storage. Unsupported keys return
`SYSTEM_OPTION_NOT_WRITABLE`; they are never silently accepted.

Writes require a current superadmin Session. API Keys, lower platform levels,
expired or revoked credentials cannot save content. Authorization remains locked
through the transaction. Changes to both announcement aliases, their revisions
and `console.options.update` audit records commit together. A failed audit rolls
back all content changes. Saving an identical value changes neither its revision
nor its audit history. A conflicting expected value returns
`SYSTEM_OPTIONS_CONFLICT` without a partial update.

A failed COMMIT acknowledgement returns HTTP 503 / `SYSTEM_OPTIONS_COMMIT_UNKNOWN`.
This does not prove rollback. Read the current content before explicitly deciding
whether to save again; do not automatically replay a changed request. A read of
matching content does not identify which request committed it.

The database runtime role is the trusted Core application identity. It has no
direct write privilege on `core_console.options`; it executes owner-controlled,
fixed-search-path functions restricted to the public keys. HTTP authorization,
content validation and actor audit are enforced by Core's transaction. These
functions do not independently authenticate a browser actor or guarantee actor
audit for a direct SQL call by that trusted role. Browser callers receive no
database credential. Neither the functions nor this trust boundary authorize
host configuration or financial mutations.

Fresh installation creates the functions and immutable owner-only Console
revision metadata. Ordinary startup performs no DDL and rejects mismatched
schema or runtime privileges. A new installation is not an upgrade procedure
for existing data. The historical v1 receipt retains its original predecessor
contract; do not change its identity v5 / 26-component contract to manufacture
an identity v6 / 27-component upgrade. Existing-site migration, continuous
write fencing and recovery need their own verified procedure.

Actual source, input hashes, failed attempts and acceptance scope are recorded in
the parent Recipkg project's `STATUS.md`. Router tests, actual Core HTTP tests,
browser acceptance and existing-site upgrades are separate results.
