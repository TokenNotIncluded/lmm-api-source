# Store module (task 07)

Fresh-install extension code for `wip/rust-core-go-extensions`, based on
`b83723cdac3f148140806b8025cb5360a2f6a26d`. No legacy migration, production
registration, public protocol change or deployment is included.

The [host entry point](../../app/run.go) mounts `store` and `support` only when
they are explicitly selected in the extension configuration; the default
selection remains identity-only. The interfaces below describe the isolated
module boundary, not a claim that a running host has selected the module or
completed Rust funds integration. [Support](../support/README.md) retains its
own storage and authority boundary.

## Integration boundary

`Service` implements the host's `Name() string` / `Handler() http.Handler`
contract. Integration must supply dedicated extension database pools, then wire:

```go
auth, err := rustauth.New(coreClient) // handle each error before continuing
storeRepo, err := store.NewPostgres(storeDB)
commerce, err := store.New(storeRepo, auth, nil) // money deliberately disabled
supportRepo, err := support.NewPostgres(supportDB)
help, err := support.New(supportRepo, auth, commerce)
// Register commerce and help with modules.New in the final integration task.
```

Do not add either module to `internal/app/run.go` in this PR. Apply each module's
`schema.sql` explicitly to a new extension database, using an installation role.
The runtime roles need only their own schema and table permissions. They must
have no core database credentials, grants, foreign keys or network access.
The SQL driver belongs to composition; these modules depend only on `database/sql`.
There is no global database, default database, startup DDL or fallback memory store.

When selected, the host mounts `/extensions/v1/store/*` and removes its service
token. The Web compatibility layer may expose public catalogue reads even when
the store funds adapter is disabled, but it marks products as paused and
exposes no payment methods; purchase, payment, refund and settlement paths
fail closed with `503`. Mutating/private endpoints require the separate
`X-LMM-User-Credential` header.
The Rust adapter accepts verified sessions only: the current protobuf has no
commerce permission scopes for API keys, so all API keys fail closed.
Personal resources require the matching user. Team management requires owner or
admin; team purchases require current `can_spend`. Platform L5/L6 is not a shop
ownership override. The funds adapter must still enforce budgets at payment time.

## API

All money uses an integer `amount_minor` plus a three-letter uppercase currency.
A price of zero must be explicitly configured; paid orders never become free.
JSON input rejects unknown fields, extra documents and bodies larger than 64 KiB.
IDs are server-derived. Lists accept `after` and `limit` (1..100), and return
`items` plus `next`. Public lists filter visibility before pagination and return
the last visible ID as the cursor only when another visible page exists.

- `POST /catalog`: `CatalogChange`; actions `create_shop`, `create_product`,
  `create_variant`, `product_title`, `product_status`, `variant_details`,
  `stock_adjust`, `quota_set`. Supply a new `key` for each distinct edit.
- `GET /shops/{shop}/products` and `/variants`: published catalog only.
- `GET /shops/{shop}/manage/{products|variants|orders}`: seller-only records.
- `POST /orders`: `Purchase` (`shop_id`, `variant_id`, `buyer`, `quantity`, `key`).
- `GET /shops/{shop}/orders/{order}`: buyer account or seller management only.
- `POST /orders/change`: `OrderChange`; actions `pay`, `cancel`, `fulfill`,
  `request_refund`, `reject_refund`, `refund`, `settle`.

Shop owner and variant stock mode are immutable. Products start unpublished.
Create the product, add variants, receive real stock, then publish. A variant
price edit never changes existing order price, currency, quantity, title or owner.

## Inventory and order rules

`stock` is real physical quantity, including reserved items. `reserved` holds
units assigned to orders before delivery. `claimed` is sales capacity already
allocated. `quota=-1` is unlimited; any other value is an absolute sales ceiling,
not a request to create inventory. Setting a quota below `claimed` blocks new
purchases but does not alter accepted orders. Unlimited -> 5 -> 0 -> unlimited
changes only quota. Stock adjustments require a reason and a replay-safe key.

Allocation and order creation commit together under a database transaction lock
per shop. Locks work across independent connection pools/processes. Different
shops do not share the same lock scope. Every query and primary key is shop-scoped.

Paid order states: awaiting_payment -> payment_pending -> paid -> fulfilled ->
settlement_pending -> settled. A known payment rejection cancels and releases the
reservation. An unknown result retains both pending state and reserved inventory.
Cancellation is permitted only before payment starts, or before delivery of a
free order. A paid buyer requests a refund instead of cancelling directly.

A refund request blocks delivery and settlement. Seller approval enters
refund_pending. Before delivery, a successful full refund releases reservation
and claimed quota. After delivery it does NOT create stock or reset consumed quota.
A real returned item needs a separate audited stock adjustment. A post-settlement
refund carries the settlement reference to Rust for controlled reversal.
This version supports one full refund decision per order, not partial refunds.

## Funds are unavailable until Rust implements the port

`DisabledFunds` is the default, including a typed-nil dependency. Paid purchase,
payment, refund and settlement return `payments_unavailable` (HTTP 503), never a
fake receipt. Paid purchase fails before reserving inventory. No new money RPC
or public protobuf is invented here; core ledger code alone does not provide
a working store funds adapter.

Before enabling `Funds`, the Rust implementation MUST:

1. Atomically persist the unique operation key, immutable command, outcome and
   ledger effect. Reuse must return the original result; changed commands conflict.
2. Recheck the current credential, source account, team authority, budgets,
   currency, amount, order authorization and linked payment/settlement references.
   The service credential alone must never authorize arbitrary transfers.
3. Use an order escrow/controlled allocation: payment is not immediate seller
   payout; settlement releases that order's allocation. Refund before settlement
   reverses it; refund after settlement must perform a controlled reversal.
4. Return the exact command hash, operation key and durable reference. A definitive
   rejection guarantees no effect and no later commit. A timeout is unknown.

The hash is SHA-256 of Go JSON encoding of the one-element array `[MoneyCommand]`,
using declared struct field order and omitting the two empty reference fields.
This is a local adapter contract, NOT a new cross-language protocol; the future
protobuf adapter must map this frozen command to the canonical Rust contract.

Go saves the operation before the RPC and holds no SQL transaction during the
RPC. Retrying after response loss, process restart or local commit failure uses
the same key and frozen payload. Unknown results return HTTP 202 with
`money_result_pending`; retry the same action. No user credential is persisted.
Known rejected operations are terminal under that key; they must not be retried
under a fresh key to hide a rejected decision. New purchases need a new order key.

## Legacy Web and historical contracts

The [Web client](../../../../../apps/web/src/features/store/api.ts) still requests the
old `/api/store` API. Its guest pickup/email, delivery templates, partial refunds,
external gateways and analytics are not supplied by this module. Retained pages
and tests do not establish backend integration or payment-provider acceptance.
The [delivery parser](../../../../../apps/web/src/features/store/delivery-template.ts)
interprets only a valid v1 envelope matching the frozen order template; other
contents remain literal text. This is a legacy Web contract, not a new delivery API.

Account carts and favorites use `/api/store/cart`, `/api/store/favorites` and
`POST /api/store/collections/cleanup`. Every request uses the current verified
account; references contain no frozen price or reserved inventory. Cart PUT
sets an absolute SKU quantity, so the browser's explicit guest-cart transfer can
retry without adding the quantity twice. Lists resolve current visibility and
availability. Paused products remain saved; privacy changes hide their content,
and cleanup removes inaccessible products and disabled or missing variants.
Existing store databases require the explicit
[`006_account_collections.sql`](migrations/006_account_collections.sql) migration
and its runtime grants before the updated host starts.

The seller editor uses the separate `store_visibility_supported` capability for
public, registered and private visibility. Guest checkout remains behind
`store_access_supported`. Self-purchase requires Core's negotiated
`commerce.self-purchase.v1` capability; an older Core leaves these purchases
unavailable before stock reservation. Private owners may buy draft, pending or
published products through the same terms, quantity, stock and delivery checks.
Paused, rejected and deleted products cannot open a new order.

At the current zero merchant fee, a self-purchase reserves the entire price and
captures it back to the original wallet. Core records a full same-wallet refund
against the original capture without issuing another credit or changing debt.
The [Core56 to Core58 procedure](../../../../../docs/commerce-display-rate-upgrade.md)
installs this closed ledger rule explicitly; ordinary transfers still reject
self-transfer. The reproducible
[PostgreSQL check](../../../../../scripts/test-commerce-self-purchase.py) covers
the SQL upgrade, original funds and debt, exact replay and concurrent calls.

[Extore integration](../../../../../docs/extore.md) stores
encrypted long-lived connections under a seller-scoped Extore shop, synchronizes
approved listings into the legacy merchant shop, and rejects local edits of
upstream-managed products. Configure `store.extore_key_file` and
`store.extore_redirect_uri` together. Sync never issues Extore cards or invents
stock; inventory remains a separate sealed-stock path. Browser callback cleanup
does not prove server or proxy logs are protected, and this README does not
claim production acceptance.

Legacy `merchant_store_*` schemas, writer floors, native startup baselines and
provider procedures are [fixed historical sources](https://github.com/TokenNotIncluded/lmm-api/tree/c33503f212cbcfab0695d554f57b5153cd4f3bfd/docs),
with their scope explained in the [archive guide](../../../../../docs/archive.md).
They do not authorize this branch to upgrade an old database or recover a running
service. Existing paid orders, delivery and refund obligations retain their frozen
original evidence; unknown payment results do not justify releasing holds or
sending a new money-moving request.

## Storage choices

Each entity has its own table in `lmm_store`: shops, products, variants, orders,
commands, stock_movements and money_operations. The row payload is JSONB, with a
shop-scoped primary key, explicit inventory/order checks, and query indexes.
This keeps immutable order snapshots separate from mutable catalog records.
The schema is for a new extension database; it does not migrate old
`merchant_store_*` tables.
Only whitelisted table names can reach the SQL adapter. All SQL values are bound.
Historical command/operation records must not be pruned while retries are possible.
The test memory repository exists only in `_test.go`.

## Tests

From `services/extensions`, with the repository's Go 1.25 toolchain and dependencies:

```sh
go test -race -count=1 ./internal/modules/store/... ./internal/modules/support/...
go vet ./internal/modules/store/... ./internal/modules/support/...
```

For actual PostgreSQL, create a new disposable database ending in `_test`, with
no `lmm_store` or `lmm_support` schema. The separate test module supplies its own
pinned database driver and does not edit the host's dependency files:

```sh
cd internal/modules/store/pgtest
LMM_STORE_TEST_DSN='postgres://.../commerce_test?sslmode=disable' go test -mod=mod -race -count=1 ./...
```

The integration test uses two independent pools for concurrent purchases and
payment replay, checks a real SQL stock constraint, and exercises support storage.
It refuses other database names and pre-existing commerce schemas. Test cleanup
drops only the schemas created by that test. Do not point this at production.

`VERIFICATION.md` records what was actually run, separately from these runnable
but environment-dependent acceptance tests.

### Durable pickup email

Pickup email is opt-in and requires the complete `[store.pickup_mail]` TLS SMTP
configuration documented in the extension host README. A product with
`email_pickup_link=true` requires a validated `pickup_email` at checkout. The
order, its frozen encrypted email payload and pickup digest are committed in the
same shop transaction. Retrying the same order with another address conflicts.
Existing plaintext pickup tokens remain valid; new mail tokens are stored as a
digest, with the recoverable token only in the encrypted mail command. The
configured HTTPS origin supplies the link, never a request Host header.

The existing `lmm_store.commands` table is the outbox; no DDL or new runtime
grants are required. The normal bounded host recovery worker claims an attempt
under the shop PostgreSQL lock, releases the transaction before SMTP, then
persists the result only for the matching attempt. It observes host admission,
uses a ten-second total network deadline and joins before database close.
Outcome persistence has a separate three-second cancellation-independent budget.
The dedicated raw 32-byte mail key must survive restart and remain distinct from
stock/Extore keys and host/Core credentials. Losing it blocks recovery; startup
never generates a replacement or rewrites historical pickup records.

Mail states in the private command are:

- `pending`: frozen and waiting for an eligible paid/free order.
- `retry_wait`: known pre-acceptance failure, retried after a one-minute backoff.
- `dispatching`: a persisted attempt owns the bounded network operation.
- `accepted_by_smtp`: the final SMTP DATA response accepted the message; this
  does not prove inbox delivery. A later disconnect cannot revoke acceptance.
- `rejected`: explicit SMTP 5xx; retained without automatic retry.
- `unknown`: final DATA ACK may have been lost, or an unfinished attempt remained
  after a crash. Never automatically resent, even with the same Message-ID.
- `canceled`: an unsent order was canceled or fully refunded.

After thirty seconds, a stale `dispatching` attempt becomes `unknown` instead of
being reclaimed for sending. This can conservatively include a crash before
SMTP actually began. A stable Message-ID supports diagnosis; SMTP does not offer
an exactly-once guarantee. An operator must resolve unknown/rejected messages
before any separately authorized resend; there is no automatic resend endpoint.
Paused/refunding orders do not become eligible through email recovery. Clicking
a received link still enforces current order/refund, frozen login and pickup-code
checks; sending an email grants no Core identity, money or inventory authority.
Do not log recipients, tokens, message bodies or SMTP/provider error strings.
