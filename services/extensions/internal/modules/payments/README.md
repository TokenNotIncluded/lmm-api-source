# Go payment extension

Implementation: `main` in `TokenNotIncluded/lmm-api-source`.

**Local WIP, not production-enabled.** Go owns provider orders and callback
processing; Rust independently accepts provider proof and owns all ledger
mutations. Construction does not enable live channels or install a schema.
[PROTOCOL_REQUIREMENTS.md](PROTOCOL_REQUIREMENTS.md) separates implemented
contracts from remaining acceptance work. [VERIFICATION.md](VERIFICATION.md)
records current Go/PostgreSQL checks and the completed simulated Rust/Go payment
recovery test, separately from historical task 10 results.

## Ownership and construction

Go owns provider orders, callback evidence and processing state. Rust owns user
and team permissions, immutable payment intents, conversion quotes, balances,
refund holds and ledger receipts. Private module routes use an explicit Rust
`account_id`; it is not a team ID or the creator's user ID. Native browser quotes
instead use `PreparePersonalTopup`, which derives the actual personal account in
Core. Go never chooses a substitute account, calculates credit units or accesses
a core database.

`Service` implements the host's `Name` / `Handler` interface. Inject a dedicated
`Postgres`, `rustbridge.Bridge` and configured adapters using `New`. Construction
starts no worker and makes no provider call. The completed joint test constructs
this service directly; it does not verify host registration, public proxy routes
or production configuration. Those remain separate integration work. Other
extensions and the Rust core must remain available if this module fails.

Install `schema.sql` explicitly into a **new payment-only PostgreSQL database**.
Its login role must have no core database/schema access. `NewPostgres` checks the
expected database name, payment marker and absence of `core_*` schemas, but those
checks do not replace database permissions. It performs no automatic DDL. The
payment package uses `database/sql`; the composition root supplies the driver.

## Routes and execution

Mount `Handler()` through `modules.New` for private user routes:

| Method / module-relative route | Result |
| --- | --- |
| `POST /orders` | Rust intent, durable order and queued checkout; returns 202. |
| `GET /orders/{id}` | Local order status after current Rust authorization. |
| `POST /orders/{id}/refunds` | Requests a Rust hold before a provider refund. |

User routes require `X-LMM-User-Credential`; a host service token is not user
permission. Mutation routes also require `Idempotency-Key` (16–128 letters,
digits, underscore, hyphen or dot). Credentials are never stored in orders,
events or jobs. Authorization is repeated on retries. Bodies reject unknown and
duplicate fields, fractional minor units and trailing JSON.

```json
{"account_id":99,"channel_id":1,"currency":"USD","amount_minor":1000}
```

Expose `WebhookHandler()` on a **separate callback-only ingress**, at
`/callbacks/{channel}`. Do not expose user routes or give providers the host
credential. Apply request/concurrency limits and disable body/query logging;
ePay signatures and query credentials are sensitive. Internal host limits do not
replace public callback ingress limits.

A bounded, cancelable host worker calls `Work(ctx, limit)` with limit 1–100.
Each iteration drains stored events and performs at most one job per selected
order. No unbounded goroutine or retry loop is created by this package.
`Reconcile(ctx, orderID)` is a trusted worker operation, not a public endpoint.
It can retry a conflicting settlement with its original accepted proof and key.
After a confirmed Core release frees capacity, `Work` persists that same retry
with the release receipt, so normal host scheduling can resume it after restart.
Neither path requeues a provider refund POST or overrides Core authorization.

## State and recovery

```text
callback -> parse / provider API observation -> resolve frozen Rust intent
         -> Rust proof lookup or independent verification + durable acceptance
         -> durable Go event inbox -> order row lock -> durable job
         -> controlled Rust RPC with accepted proof ID + exact original bytes
         -> validated durable receipt -> public status
```

Callback success requires both a Rust durable proof receipt and a saved Go inbox
event. It is **not account credit**. Go checks the receipt's intent, event ID,
payload SHA-256, provider/merchant/environment/transaction and acceptance time.
Missing dependencies, unsupported events, unknown orders and invalid bindings
return non-2xx so the provider can retry. They are never falsely acknowledged.
Original evidence remains private; public status exposes no signatures or jobs.

| Event state | Meaning |
| --- | --- |
| `pending` | Saved with accepted proof, or an unsigned API observation awaiting proof/retry. |
| `done` | Applied to local state or a durable job, not necessarily posted by Rust. |
| `review` | Conflicting identity, amount or event meaning. |

| Job state | Meaning |
| --- | --- |
| `pending` | Waiting for a bounded attempt or dependency recovery. |
| `running` | Claimed for 30 seconds; only the matching claim may save its result. |
| `done` | Provider step or validated Rust receipt completed; provider steps do not settle funds. |
| `review` | Missing independent evidence, conflicting receipt or unsafe provider retry. |
| `rejected` | Explicit durable Rust rejection. |
| `fenced` | An earlier refund prevents local credit replay; Rust must enforce this atomically too. |

| Public order state | Meaning |
| --- | --- |
| `checkout_pending` / `awaiting_payment` | Checkout not ready / waiting for payment. |
| `credit_pending` | Provider paid; no confirmed Rust credit. |
| `credited` | Rust posted the original credit, not a statement of current spendable balance. |
| `refund_pending` | Awaiting hold, provider evidence or Rust refund confirmation. |
| `partially_refunded` / `refunded` | Rust confirmed the corresponding refund records. |
| `review` | Order-level terminal-state conflict. |

`core_credit_confirmed` refers to the original credit. Clients must read current
balances from Rust, including after refunds. A job in review leaves the user
status pending instead of inventing success; inspect private jobs for its reason.

Event deduplication uses provider + merchant + environment + event ID. Changed meaning or original payload bytes under one event ID are rejected. Distinct paid events for one transaction
use the same credit job. A database unique index prevents that transaction from
binding to two orders. Late failure cannot erase terminal success. Missing proof can be upgraded to a
Rust-issued proof only for identical original bytes under the same event ID. A
second accepted event may repair the same semantic job only when its immutable
payment/refund bindings agree. Completed, rejected and fenced jobs never reopen.
Review entries can recover only when missing proof becomes durably accepted.

PostgreSQL row locks protect order updates; network calls run outside database
transactions. Durable claims recover after restart, and expired workers cannot
overwrite newer claims. After a lost Rust response, query the same scoped receipt
before retrying the original key. Timeout never creates a receipt. Receipt checks
require matching intent/action key, known state, a 32-byte fingerprint and a
journal ID for posted operations. The canonical fingerprint format remains a
protocol requirement, not a locally invented cross-language contract.

## Refunds

The user refund flow is `hold_pending -> held -> awaiting_evidence -> committed`.
Rust first holds funds. Go then sends a provider refund with a stable request key.
Only independently verified success plus Rust `REFUND_COMMITTED` completes it.
A hold may be released only after Rust returns `REFUND_RELEASED`. Rust requires
the signed failure to match the original registered dispatch and rejects a
failure that cannot exclude another accepted pending/successful attempt.
Timeout, missing callback, API error or unsigned provider success never releases
funds. Lost hold responses use receipt lookup; background jobs never save user
sessions. Scoped synthetic process tests cover credit and refund recovery;
their exact source, scope and results are recorded in the parent STATUS.md and
[independent fixtures](../../../../../tests/payments/independent_a05/README.md).

Partial refunds have separate IDs and request keys. Local locked checks cap the
cumulative requested amount; Rust enforces the authoritative cap and holds.
Stripe dashboard refunds use the external-refund path. A signed late success for
the same released refund still goes through Core's dispatch, capacity and ledger
checks. Capacity conflicts remain in review until the original job can settle;
successes exceeding the original payment stay unresolved. Refunds before credit
set a local fence, while Rust must
atomically prevent stale credit and reconcile overlaps with existing holds.

## Provider support and limits

Version 2 host configuration assembles Stripe, EPay, Creem, Waffo Global and
Waffo Pancake. The native wallet has authenticated channel, quote, confirmation
and checkout-status routes under `/api/user/payments/`. The complete configuration
and currency contract is in [Native wallet payment gateways](../../../../../docs/payment-gateways.md).
FastPay is retired and is not an accepted provider.

**Stripe direct accounts:** fixed-amount hosted Checkout, PaymentIntent success
and failure, refund created/updated/failed, partial refunds, authenticated lookup
and bounded refund pagination. Connect-account events are rejected in this basic
adapter. The required pinned API version has no changing default. Checkout
disables adaptive pricing, promotions and automatic tax to preserve the intent.

Rust alone holds Stripe webhook secrets and verifies the exact original body,
five-minute delivery window, merchant, frozen intent, amount and event semantics
before persisting proof. `StripeConfig.WebhookSecrets` is deprecated and must be
empty; `NewStripe` rejects nonempty values. Go forwards the unchanged bytes and
signature header. Its API-key merchant/event checks are observations, not an
authenticity assertion.

Ingress first looks up an accepted proof by intent, provider event and payload
SHA-256. This recovers a lost Go ACK or inbox write even after delivery freshness
expires. Workers also recover accepted proof by that immutable binding; they do
not re-accept aged, previously unverified signatures. Settlement requires the
Rust proof ID and exact payload. Provider mutation retries retain their key and
stop after 23 hours. API observations have `source=api`, no forged signature or
accepted proof ID, and cannot settle funds. A later accepted webhook can repair
an earlier unproved job, including while that job is in flight.

**EPay MD5 profile:** the production host uses `NewCoreEPay`. Core holds the
merchant key, signs the frozen checkout intent and independently verifies the
original GET/form callback. CNY and Linux DO LDC are separate configured units;
the wire format itself contains no currency or environment. Each namespace has
its own key and durable replay protection. Payment type, merchant, order and
exact amount must match. Go-side credentialed lookup and automatic refund
dispatch are unavailable on this Core-owned profile. The old key-bearing
`NewEPay` constructor remains for existing package fixtures, not host assembly.

**Creem:** one-time, inclusive-tax, fixed-price products in USD/CNY/EUR. Go checks
the live product against the quote before checkout; Core verifies original HMAC
evidence, environment, product, merchant metadata, order and exact amount.

**Waffo Global:** RSA-signed order creation and callbacks. A shared provider
public key alone does not bind merchant-supplied metadata to a receiving
account. Core independently inquires about the frozen order using its own
provider credentials before persisting a new proof, then rechecks the intent
inside the transaction. Go's own inquiry is still an observation.

**Waffo Pancake:** authenticated checkout through the pinned official SDK,
separate from Global's protocol. Core verifies the original timestamped RSA
notification and binds merchant, test/prod environment, store, product, order,
currency and actual paid amount. Creem, Global and Pancake automatic refunds
are not enabled by this top-up implementation.

Provider connections require HTTPS, block redirects and proxy environment use,
reject private/link-local/metadata addresses and pin checked DNS addresses.
Evidence and responses are bounded. No credential-bearing HTTP/SQL error escapes
in a user response. Adding adapters changes Go; enabling arbitrary new providers
without a Rust redeploy also requires the independent verification boundary in
the protocol handoff. No Go `verified=true` flag grants ledger authority.

## Tests and evidence

```sh
cd services/extensions
MISE_IGNORED_TOOLS=rust MISE_AUTO_INSTALL=false mise exec -- go test -mod=readonly -race -count=1 ./internal/modules/payments/... ./internal/coreclient
MISE_IGNORED_TOOLS=rust MISE_AUTO_INSTALL=false mise exec -- go vet ./internal/modules/payments/... ./internal/coreclient
```

The nested `pgtest` module uses `PAYMENT_TEST_DSN` supplied privately by an
isolated test runner. It refuses any database name except `lmm_payments_test` and
resets only that disposable schema. Never use application or production data.

`TestRustDurableProofAcrossGoRestart` requires an explicit private fixture through
`PAYMENT_CORE_PROOF_FIXTURE` and phase `recv`, `recv-lost`, `offline` or `settle`
through `PAYMENT_CORE_PROOF_PHASE`. The runner
must provision separate fresh Rust and Go databases, install Go's schema, sign
payloads outside Go and execute each phase in a separate Go process. Callbacks use a real loopback HTTP test server; they do not traverse Caddy.
The fault phases require explicit runner-owned database/UDS fault injection.
No webhook secret is passed to Go. The default test run skips this joint test. A rebuilt Rust
service object is not a Rust operating-system process restart. Consult
[VERIFICATION.md](VERIFICATION.md) for the actually executed phases and limits.

The four phases passed locally on 2026-10-12: four actual Go processes, loopback
HTTP callbacks, UDS RPC and separate synthetic PostgreSQL databases. Recovery
survived a Go inbox-write failure, an offline Rust service, a real 302-second wait
and webhook-key rotation. Two accepted proofs produced one credit receipt and
one balanced journal. This establishes that bounded recovery path; it does not
verify a real provider, the normal host, Caddy, browser flows or production.

Provider references checked during implementation:
- https://docs.stripe.com/webhooks
- https://docs.stripe.com/api/checkout/sessions/create
- https://docs.stripe.com/api/refunds/create
- https://docs.stripe.com/api/refunds/list
- https://docs.stripe.com/api/idempotent_requests
- https://pay.gggua.com/doc.html (one legacy ePay profile, not a universal guarantee)
