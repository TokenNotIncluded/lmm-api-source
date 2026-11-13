# Native payment discount management

The native wallet's discount-code page uses Core directly. Management requires
the user's current personal Session and platform admin/superadmin authority.
Imported privileged users must satisfy their persisted import policy. An API
key is not a management credential.

## HTTP contract

All paths below are relative to `/core/v1/payment-discounts/admin/codes`.
Use the base path itself for `(base)`. Responses use the existing
`{success, data}` envelope.

| Method and path | Behavior |
| --- | --- |
| `GET (base)` | Descending ID pagination; `after_id=0` starts a list, `limit` is 1–100 (default 20). Optional `keyword` matches a normalized code prefix or a case-insensitive name substring; `enabled` is an optional boolean. Data is `{items, next_after}`. |
| `POST (base)` | Create one explicit code definition. |
| `PUT /{id}` | Update the definition, including renaming the code while retaining its ID. |
| `PUT /{id}/status` | Change only `{enabled}`; preserve price and validity terms. |
| `POST /batch` | Atomically generate 1–100 cryptographically random codes. Data is `{items}`. |
| `DELETE /{id}` | Idempotent soft deletion; existing payment obligations remain. |
| `DELETE /exhausted` | Remove codes whose nonzero usage cap has been consumed. Pending reservations alone never qualify. Data is `{count}`. |

A definition contains `code`, `name`, `discount_percent`, `min_credit_units`,
`max_uses`, `enabled`, `starts_at` and `ends_at`. The percentage is an integer
from 1 through 99. Minimum Credits, usage cap and Unix-second timestamps are
nonnegative signed 64-bit integers; zero means no minimum/cap/start/expiry as
appropriate. A nonzero end must be later than the start. Monetary display
currency never changes `min_credit_units`.

Core accepts exact JSON integers or decimal strings for 64-bit inputs. Web
sends decimal strings and requires decimal strings in responses, including
IDs, timestamps, minimum Credits, caps and used/reserved counts. This preserves
values beyond JavaScript's safe-integer range. Name and percentage remain
ordinary string/number fields.

Batch input replaces `code` with `request_key` (UUID) and `count`. Core binds
the normalized request and result to the actor and UUID in the same transaction
as creation. The same request returns the original batch after reauthorization;
changed input conflicts. Web persists the pending request in Session-scoped
storage before sending it and recovers that request after a lost response.

Editing, disabling or deleting a code blocks new uses as appropriate. Existing
intents retain their frozen amounts and reservations; an accepted late payment
can still consume its original reservation. Deletion does not remove financial
history or release an unpaid order's reservation.

## Installation and offline upgrade

Component 68, `payment-discount-management`, adds immutable tombstones, batch
receipts and a private upgrade receipt. Historical component 67 SQL and its
66→67 target pin remain unchanged. Existing 67 databases use the explicit
`core-payment-discount-management-v1` procedure with the normal `UpgradeRequest`
v1 input to `lmm-core --config /absolute/installer.toml upgrade-db`.

| Binding | Exact 67 predecessor | Exact 68 successor |
| --- | --- | --- |
| Manifest SHA-256 | `8a6bed1a727ea891f270b0d78b5dbc67912a94e244661f29e3ab9541ccf94fa0` | `eeae72df806b9069e2c3bddc65c5f8271bf8b3c4b937b0de9ebb13e3ebcf8ced` |
| Catalog SHA-256 | `77dc2ffca5d5d4870452f15c7e1cbd5a29d238412bdcd4e592a388492dac321a` | `2f067e60e07c28a38fb9ea03fe36f8740da8bddd087f80e794b843da6ff6e190` |

Database identity and the distinct installer/owner/runtime/extensions roles
must describe the intended offline database. Keep writers closed through all
required transitions and preserve each original request/receipt. An unknown
commit outcome requires recovery with that operation's original request. This
procedure does not execute imports, configure providers or reopen a site.

The legacy import contract remains bound to its reviewed fresh66 target.
Finish its required imports, reconciliation and unknown-outcome recovery before
the separate 66→67 and 67→68 upgrades; the
[fresh66 handoff planner](../scripts/plan_fresh66_payment_discounts_handoff.py)
prepares only the first transition. Verify the final 68 catalog, grants and
imported data before admitting the current runtime. Native code management
does not activate historical private codes or convert legacy minimum-amount
units automatically.
