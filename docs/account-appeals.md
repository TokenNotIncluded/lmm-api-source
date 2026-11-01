# Account suspension and appeals

Core owns the suspension generation, ownership proofs, requests and review
transaction. The extensions service does not receive password hashes, Session
secrets or direct identity database authority.

## User flow

A disabled user can verify their existing password or linked browser identity.
Verification issues a ten-minute, HttpOnly `lmm_account_appeal` Cookie scoped to
`/api/user/account-action-requests`. This proof grants only appeal submission and
the corresponding receipt. It never grants a Session, API key, refresh family or
MFA bypass for ordinary endpoints. Closed accounts and privileged administrator
recovery retain their separate procedures.

- `GET /api/user/account-action-requests/appeal` returns verified context and an
  existing receipt. An absent or expired proof returns 401 without attempting
  ordinary credential refresh.
- `POST /api/user/account-action-requests/appeal` accepts `{reason}` with that
  Cookie, or `{username,password,reason}` for a disabled password owner. Writes
  require the configured Origin and JSON. A username alone is never proof.
- Submissions have 5–2000 characters and at most 8192 UTF-8 bytes. One request
  remains pending per suspension generation. Concurrent submissions return its
  original reason; retries cannot overwrite evidence. Reusing the same proof
  with different evidence conflicts.

Only the explicitly published `public_reason` for the matching suspension is
shown. The administrator's required private `reason` and later review `note`
remain private. Historical suspensions without a public explanation are shown
as having no published reason; the service does not invent one.

## Administrator flow

`GET /api/account-action-requests` supports `status` (`pending`, `approved`,
`rejected`, `superseded`, `all`), `page` and `page_size` (maximum 100). Its existing
array envelope also provides `pagination` with the total count.

`POST /api/account-action-requests/{id}/{approve|reject}` requires a current
administrator Session and `{note,expected_revision,expected_auth_version}`.
Both versions are canonical decimal strings. Reviews apply only to ordinary
users and the unchanged suspension generation. A later security change makes a
pending request `superseded`; stale or conflicting reviews return 409. Exact
replays of the original review return its receipt without another decision.

Approval enables the user and advances their identity generation in the same
transaction as the request decision and audit. It restores no revoked Session,
refresh family or API key. The user must log in again and satisfy their current
MFA policy. Rejection leaves the account disabled. Neither decision changes the
ledger or reopens a closed account.

## Installation and upgrade

The independent `account-appeals` component is component 42. Fresh `init-db`
installs it. Existing exact 41-component PostgreSQL 18 installations use the
explicit offline `core-account-appeals-v1` procedure; the fixed 32-component
predecessor remains supported by the updated functional-completion procedure.
Startup never installs the component. Use the matching release manifest and
measured catalog target, the existing maintenance/fenced-role procedure, then
the installer grant script and runtime `check-db`.

Runtime gets SELECT/INSERT on requests and only the review columns for UPDATE.
Flow UPDATE is restricted to `request_id`; runtime cannot rewrite ownership,
proof digests, generations or reasons. Upgrade receipts remain owner-only.
The offline upgrade preserves predecessor rows, sequences, owners and existing
ACLs, and does not automatically expand runtime authority.

Local database and HTTP verification is distinct from browser acceptance and
production rollout. Resolving the reported account's actual suspension and
reviewing its case requires the production operator; this implementation alone
does not close that part of issue #971.
