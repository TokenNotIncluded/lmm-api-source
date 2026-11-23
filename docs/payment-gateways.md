# Native wallet payment gateways

This guide defines wallet gateway configuration, payment evidence and recovery.
Current implementation gaps and acceptance work belong in [project progress](progress.md).
Wallet top-ups, subscription purchases and merchant-store orders have separate contracts:
enabling a top-up provider does not enable its subscription lifecycle or store checkout.
Core owns wallet balances and subscription entitlements; Go owns provider communication and order recovery.

Native [subscription purchases](../services/core/src/subscriptions.rs) spend Core
wallet balance and issue consumption entitlements. Store balance purchases use the
separate [Commerce bridge](../services/extensions/internal/modules/store/rustfunds/adapter.go);
explicit free orders carry no funds transfer. Neither path grants authority to a
provider's recurring billing or external merchant checkout.

## Provider and unit contracts

The native wallet has six provider protocols. EPay supplies three channel presets;
the first five protocols cover the seven choices in the
[legacy wallet templates](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/web/src/features/system-settings/integrations/payment-method-templates.ts).
Coinbase Business Checkout is an additional protocol. FastPay is retired.

| Protocol / channel | Settlement unit | Checkout and Core proof admission |
| --- | --- | --- |
| Stripe | USD cents | Hosted Checkout; Core verifies the original HMAC callback. |
| EPay Alipay / WeChat Pay | CNY fen | Core signs the frozen intent and verifies the GET/form callback; Go receives no MD5 merchant key. |
| Linux DO Credit over EPay | Hundredths of LDC | The `epay` payment type uses `https://credit.linux.do/epay/pay/submit.php` through an explicit form POST. LDC has a separate conversion policy and is a virtual unit. |
| Creem | USD / CNY / EUR minor units | A configured active one-time product fixes the amount. Checkout checks its price, currency, environment and inclusive tax; Core verifies HMAC and product/order/amount binding. |
| Waffo Global | USD cents | Official hosted order protocol. Core verifies RSA and independently queries the frozen order using its own provider credentials before accepting new proof. |
| Waffo Pancake | USD cents / CNY fen | Official SDK checkout. Core verifies timestamped RSA, environment, merchant, store, product, original order and actual paid amount. |
| Coinbase Business Checkout | USDC, quoted as USD cents at 1:1 | CDP-signed checkout request; Core admits a signed completed payment bound to the frozen intent and exact amount. |

Only a Core ledger receipt confirms wallet credit. A provider API response, return
page or Go order status cannot supply that authority. Site finance includes native
payments backed by a Core credit journal, reports cash currencies separately and
LDC as a virtual unit, and does not add unlike units.

## Configuration and ownership

Use [Core payment configuration](../config/core-payments.example.toml) for proof
verification and conversion, and [Extensions payment configuration](../config/extensions-payments.example.toml)
for outgoing adapters. Both use `version = 2` with explicit provider and environment.
Version 1 Stripe configuration remains accepted; isolated test profiles are laboratory configuration.

`enabled` admits new wallet orders. `subscription_enabled` separately admits new
EPay, Stripe, Creem or Pancake subscription purchases and defaults to false.
Disabling either switch preserves issued orders and provider callbacks. Coinbase
and Global do not accept this subscription switch. Creem subscriptions can omit
the wallet product when wallet checkout is disabled; their product and exact
price come from the published plan revision instead.

The two sides must agree on channel ID, provider, merchant, environment, currency
and product/store/payment-type binding. A channel ID identifies an immutable policy.
When changing that policy, allocate a new ID and retain the old channel's verification
material for outstanding callbacks. Setting the old Go channel's `enabled = false`
removes it from new checkout selection while allowing recovery.

Core owns `credits_per_minor`, `min_amount_minor` and `max_amount_minor`. It freezes the checked integer
quote `amount_minor × credits_per_minor` on the intent. The ledger unit is
`credit_500k_usd`; one USD cent corresponds to 5,000 credit units. Example non-USD
rates are illustrative administrator policy, not live exchange rates or defaults.

| Configuration material | Owner and use |
| --- | --- |
| `webhook_secret_file` | Core only: Stripe/Creem/Coinbase webhook secret, EPay merchant key, or Waffo provider public key. The field name is historical. |
| Waffo `inquiry_api_key_file`, `inquiry_private_key_file` | Core uses both in test and live environments to query the fixed official endpoint. Callers cannot supply a query URL or alleged result. |
| Outgoing API keys and private keys | Go uses only those required by its provider adapter. Coinbase uses a CDP key name and P-256 PEM private key. |
| EPay checkout signing | Core's `GetEPayCheckout` returns the method and signed URL for the existing intent; Go has no merchant-key setting or credentialed lookup. |
| RPC credentials | Separate service credentials authorize narrow operations; they do not replace the user's credential or provider proof. |

Waffo inquiry runs before the database transaction; Core rechecks the intent inside
the transaction before persisting proof. Standard CNY EPay uses GET checkout and
requires `sign_type=MD5` on callbacks. Linux DO Credit uses POST with signed query
fields as form inputs; its callback may omit `sign_type`, but a supplied value must be `MD5`.

Pancake's customer portal is available when either wallet checkout or subscription
admission is enabled for the store. Disabling wallet checkout alone must not hide
subscription management. Currency aliases may share the portal only for the same
merchant, environment and store; ambiguous stores require explicit configuration.
The portal URL is issued for the current authenticated user and is never cached.

Secrets must be bounded private regular files owned by the appropriate service or
root, with mode `0400` or `0600`. The Go provider document is also private. Keep
verification material, original callback signatures, provider API keys and checkout
bearer URLs out of public status, logs, examples and browser configuration.

To assemble payments, provision the dedicated payment database and restricted Go
role, select `payments` in `host.modules`, enable `host.web_compat`, and configure
`[core].payment_token_file` plus `[payments]` storage and `config_file`.
Enable `callbacks_enabled` with its callback ingress. Core requires its own payment
configuration, narrowly granted RPC permissions and payment runtime privileges.
The example leaves refund permissions disabled; grant them explicitly when enabling refunds.

Core's `proof_ingress` and `credit_evidence` permissions authorize native intent
proofs and credits. Historical top-ups instead require `legacy_proof_ingress` and
`legacy_credit_evidence`, both defaulting to false. Existing configuration files
remain valid, but deployments that settle historical orders must explicitly grant
these new permissions after reviewing that settlement scope. Native permissions
never imply either legacy grant. `proof_read` still permits read-only proof lookup
for both paths; recovering an already admitted legacy proof still requires
`legacy_credit_evidence` to credit it. See the
[legacy settlement contract](legacy-payment-obligations.md#内部-rpc-与旧订单回调).

### Exact minimum for credit-quantity wallet payments

Core's `credit_quote.min_credit_units` is an optional positive int64 admission
limit in `credit_500k_usd` units. It defaults to 1 for existing configurations;
500000 requires at least USD 1 of wallet credit. Core checks the exact quantity
before fiat rounding and discount calculation. A fiat `min_amount_minor` cannot
substitute for this rule. The field is not added to the immutable quote stored
with an admitted intent, so a later minimum increase does not reprice or reject
recovery of that intent.

The service-authenticated `GetTopupConfiguration` RPC uses `read_user` and reads
the current Core policy without a database operation. Extensions matches each
result against its actual provider binding and quantity mode before advertising
a wallet channel. It forwards the exact minimum as a decimal string; it does not
copy the pricing policy. Deploy a matching Core before enabling the new
Extensions wallet catalogue: an unavailable policy RPC fails closed. Saved
browser requests and existing order recovery remain independent of that new
catalogue admission.

### Shared-store Pancake callbacks during native rollout

A provider store can deliver `order.completed` to both the old Go deployment and
native while each keeps its own order namespace. Configure optional
`[[payments.external_legacy_pancake_owners]]` entries in the private Extensions
runtime TOML with exact `store_id` and `environment` (`test` or `live`). No entries
means the existing local legacy ingestion and Core permission checks remain.

An entry declares that `WAFFO_PANCAKE-` and `WAFFO_PANCAKE_SUB-` orders in that
store/environment are still owned by the other deployment. The original
`/api/waffo-pancake/webhook/{test|prod}` handler uses the SDK's fixed environment
public key and timestamp window before acknowledging those completed events.
The signed store, mode and nonempty old reference must match. This acknowledgement
happens before any durable inbox or Core call and conveys no payment settlement.
Other events and namespaces retain their original processing and retry behavior.

`native_product_ids` is an optional list of **exclusive** native SKUs: a signed
`lmm_product_id` in that list with an old reference is a conflict (HTTP 409).
Omit the list or use `[]` for shared products; putting a shared SKU in the list
would also reject the old deployment's legitimate callbacks. Older callbacks
without product metadata can be acknowledged only under the explicit old
namespace ownership entry. Signing keys, namespace overrides and broad unknown
order acknowledgements are not configurable.

Native `top_` and `sub_` references always follow their existing Core/frozen-order
paths, including after product rotation; unknown native references stay retryable.
The same merchant/store/product can therefore be reused without publishing a new
SKU. Wallet checkout uses its frozen Core amount and provider binding, and
subscription checkout uses its frozen price and return URL. Set a nonempty native
wallet `return_url` for new orders to override the product's default success URL;
existing orders retain their own frozen values. This routing configuration does
not enable legacy settlement grants, subscriptions or refunds.

See [Core runtime configuration](../config/core.example.toml),
[Extensions runtime configuration](../config/extensions.example.toml) and
[database provisioning](../deployment/postgres/README.md). Ordinary startup performs no DDL.

## Owner-managed EPay / Linux DO Credit settings

The owner can edit an existing LDC connection in the payment settings UI when it
is disabled and has no payment or external subscription history. Core checks
this condition again on save, including legacy bases, proofs, events and refunds.
Ordinary administrator sessions cannot read or change this configuration.

| Setting | Management contract |
| --- | --- |
| Endpoint, notify URL, return URL | Editable for the eligible LDC binding; Core validates the URLs and LDC policy constraints. |
| Merchant key | Write-only replacement. Leave blank to retain the current key; reads return only `key_configured`, never the secret. |
| Merchant, environment, currency, payment type | Fixed policy; the editor cannot change them. |
| `credits_per_minor`, amount limits, `enabled` | Fixed policy; editing the connection does not change conversion or enable payments. |

Save uses `expected_revision` and verifies the result by reading it back.
A conflict or unknown save outcome requires an explicit fresh read; it does not
automatically replay the write. The Core endpoints are
`GET /core/v1/payment-gateways/epay` and
`PUT /core/v1/payment-gateways/epay/{id}`; the browser uses the existing
`/api/option/epay/config` bridge. See the [configuration implementation](../services/core/src/payments/configuration.rs).

For editable installations, keep the base payment TOML in a persistent directory
owned and writable by the Core service, with directory mode `0700` and base file
mode `0600`. Back up and retain that directory as one configuration set:
the base file, `<base-filename>.epay.json`, `<base-filename>.epay.lock`, and
versioned `epay-key-<channel-id>-<random>.secret` files. Core writes private files
with mode `0600`; deployment cleanup must preserve these files.
The overlay binds the exact base bytes by SHA-256, so even reformatting the base
changes its identity. Do not overwrite the running overlay or replace its base
during an ordinary binary deployment; changing the base requires a coordinated
configuration migration. See [persistent storage](../services/core/src/payments/configuration/storage.rs).

`min_amount_minor` is optional in the Core channel configuration and defaults to
`1`; it must be positive and no greater than `max_amount_minor`. It applies only
to the final payable amount of a new wallet intent or discount quote, after any
configured discount. Existing intents, their checkout and external subscriptions
keep their original contracts; changing the minimum does not reprice them.

The imported legacy policy for LDC channel `5004` retains
`enabled = false`, `min_amount_minor = 10000`, `credits_per_minor = 500`, and
`max_amount_minor = 9000000000000`. LDC minor units are hundredths: the minimum
therefore represents 100 LDC and 5,000,000 wallet credit units. The maximum is a
technical numeric ceiling, not a recovered legacy business limit. These values
do not enable legacy group, preset or coupon discounts; those require a separate
review before admitting new orders.

The native installation imported this disabled channel on 2026-11-18, after the
separate stage, prepare and activate operations all completed successfully.
Core now loads `/var/lib/lmm/core/config/payments.toml` from a Core-owned
`0700` directory; its base and initial key files are `0600`. Core and Extensions
readiness checks passed with deployed application revision `72392577`.
Existing gateway settings and funds permissions retained their original values,
including Go channel `5002`'s existing `subscription_enabled = true`.
Channel `5004` remains disabled for wallet and subscription purchases.
Real administrator read/save/reload acceptance is still pending; see the
[deployment receipts](reports/native-channel-routing-20261118.md#epay-5004-实际导入回执).

## External subscription checkout

Publish the provider product/price against a specific Core plan revision before
enabling new purchases. The browser's subscription price editor writes Core's
admin price endpoint; the buyer confirms its frozen order before Go creates a
checkout. Wallet products, conversion rates and paid wallet notifications do not
grant subscription entitlements.

| Provider | Core plan price suffix | Browser order prefix | Callback prefix |
| --- | --- | --- | --- |
| Stripe | `stripe-prices` | `/api/user/payments/subscriptions/stripe/orders` | `/callbacks/subscriptions/stripe/{channel}` |
| Creem | `creem-prices` | `/api/user/payments/subscriptions/creem/orders` | `/callbacks/subscriptions/creem/{channel}` |

The public price suffix follows `/core/v1/subscriptions/plans/{id}/`; publishing
uses `/core/v1/subscriptions/admin/plans/{id}/` and requires an administrator
Session. Use the separate subscription callback, rather than the wallet callback.
Core's historically named `subscription_epay_*` permissions govern the external
subscription RPCs for EPay, Stripe, Creem and Pancake. New subscriptions remain
disabled until their own admission switch and schema are provisioned.

Creem validates the current product, recurring period, exact currency/amount and
inclusive tax before its first checkout POST. Recovery keeps the original order
reference and never repeats an ambiguous POST. Only signed `subscription.paid`
evidence grants a cycle; checkout completion alone does not. Scheduled renewal
cancellation waits for provider lifecycle evidence to update Core state.

Existing databases require the matching reviewed Core upgrade and the explicit
Go `upgrade-payments-creem-subscriptions-db` procedure. Its installation-owner
connection and fenced runtime role are separate from normal application startup.
Fresh payment installations include the additive subscription tables and narrow
runtime grants. Do not run fresh installation against an existing payment database.

## Refunds and recovery

The native [Stripe](../services/extensions/internal/modules/payments/stripe.go) and
[Waffo Global](../services/extensions/internal/modules/payments/waffo_refunds.go)
adapters support full and partial refund dispatch.
Other native profiles do not expose automatic refunds in their normal configuration.
Unsupported requests are rejected before a refund hold or provider write is created.
Pancake additionally admits signed external refund callbacks against the original
payment, store, product and actual charged/refunded amounts. Its business refund
ID prevents a redelivered refund from debiting twice, and Core enforces cumulative
limits. Automatic ticket submission remains disabled. Callbacks missing a positive
actual `refundedAmount`, including failed refunds without that evidence, remain
unsupported rather than substituting a requested amount or price snapshot.
Subscription entitlements and store-order refunds remain separate business operations;
a wallet refund adapter does not authorize either of them.

Core implements hold, dispatch registration, commit, release and external-refund
operations under `refund_user` / `refund_evidence` permissions. `ReleaseRefund` requires the trusted evidence
and original dispatch binding checked by Core; a Go failure flag or an unbound failed
refund cannot release another operation's hold. The implementation is in
[`payments/rpc.rs`](../services/core/src/payments/rpc.rs) and
[`payments.rs`](../services/core/src/payments.rs).

The worker records `UNKNOWN` before refund provider I/O. After a crash, lost response
or timeout, it retains the original order, refund, hold, dispatch and request key.
An ambiguous refund is reconciled by its original provider identity, without sending
a second refund POST. API observations alone cannot commit or release Core funds.
After a submitted or ambiguous refund, a durable worker task queries the original
order at bounded intervals. It survives a Go service restart and stops after the
original refund reaches a Core-confirmed terminal state. An unsupported lookup
requires review; a missing signed proof keeps the hold and query task pending.
Checkout retries also respect the adapter's replay window; no assumed retention period
authorizes a new operation. Recovery behavior is defined in
[`worker.go`](../services/extensions/internal/modules/payments/worker.go).

## Coinbase Business Checkout

A quote of 1,000 USD cents creates `10.00 USDC` of checkout value. This explicit
integer conversion is shown before checkout and is not a live exchange-rate lookup.
Go's `api_key_file` holds the CDP key name; `private_key_file` holds the P-256 PEM key
used for short-lived request JWTs. Core separately holds the webhook subscription
secret. The configured `merchant` is a stable account label matching on both sides.

Use separate channel IDs, subscription secrets and callback URLs for test and live.
Test requests use `https://business.coinbase.com/sandbox/api/v1/checkouts`; live
requests use `https://business.coinbase.com/api/v1/checkouts`. Provision the sandbox
subscription with `sandbox = "true"`. Do not reuse a secret across environments:
the integration does not rely on an independently signed sandbox-mode field.

Subscribe to `checkout.payment.success`. Core requires `status = "COMPLETED"`,
`currency = "USDC"` and the exact frozen amount. The webhook ID is the checkout ID;
local event identity also includes the event type. The adapter accepts
`X-Webhook-Signature` or the compatibility header `X-Hook0-Signature`; dual headers
must agree, and repeated headers are rejected.

The adapter accepts `0.01` through `100000000` with at most two fractional digits;
Core's configured maximum may be lower. Checkout retains its original UUIDv4 key.
Its replay window is zero, so an ambiguous write stays in reconciliation rather
than being retried after an assumed provider retention period. This is the Business
Checkouts integration; it does not use the older Commerce Charges API or dispatch
Coinbase refunds and subscriptions. Setup references:
[authentication](https://docs.cdp.coinbase.com/coinbase-business/authentication-authorization/api-key-authentication),
[checkout](https://docs.cdp.coinbase.com/api-reference/business-api/rest-api/checkouts/create-checkout),
[webhook events](https://docs.cdp.coinbase.com/coinbase-business/checkout-apis/webhooks),
[signature verification](https://docs.cdp.coinbase.com/webhooks/verify-signatures) and
[sandbox](https://docs.cdp.coinbase.com/coinbase-business/checkout-apis/sandbox).

## Browser and edge contract

The wallet uses a personal Core Session. Core derives the economic account; the
browser cannot select an account ID, conversion rate or credited amount. Only
configured, enabled channels are offered.

| HTTP route | Purpose |
| --- | --- |
| `GET /api/user/payments/channels` | Available channels, settlement units and any fixed Creem amount. |
| `POST /api/user/payments/quote` | Freeze the channel, amount and original request key in a Core intent; no provider checkout or wallet credit. |
| `POST /api/user/payments/checkout` | Confirm the original order and exact credit quote, then queue checkout. |
| `GET /api/user/payments/checkout/{top_id}` | Owner-authenticated checkout link and order state. |
| `GET /api/user/payments/orders/{top_id}` | Minimal status without a checkout URL. |

IDs and money use canonical decimal strings and integer arithmetic. The UI shows
the exact currency and credit quote before confirmation and opens the gateway on a
user click. `checkout_method` selects a GET link or POST form. Provider-paid and
Core-credited states remain distinct; a return URL is never payment evidence.

Save the original request key before the first write. Timeout, lost response or
reauthentication must not create a replacement operation. Writes disable automatic
authentication-refresh replay; status uses bounded GET polling. Only the matching
Core-confirmed receipt displays completion. Wallet reads are documented in
[wallet history](wallet-history.md).

Route the whole `/api/user/payments/` prefix to Extensions ahead of Core's `/api/user/`,
retaining the user's Bearer header. Provider callbacks use `/callbacks/{numeric_channel_id}`
with protocol-specific GET/POST, exact query/body/signature bytes and no browser or
service credentials. Apply request limits without sensitive query/body logging.
See [edge compatibility](edge-cutover-compatibility.md).

## Schema and implementation references

The installed component set comes from the binary's `schema-manifest`, not a count
copied into this guide. New installation and existing-database upgrades follow
[Core installation boundaries](core-migration.md#安装权限与结构升级).
Gateway schema changes do not reprice old intents, import payments or enable channels.

Historical native databases may require the preserved `core-payment-gateways-v1`
[exact 28 → 29 procedure](payment-gateways-upgrade.md), or the
`core-coinbase-payments-v1` [exact 31 → 32 procedure](schema-coinbase-payments-upgrade.md).
Those guides describe specific predecessors and executors, not the current schema
size. A different predecessor needs its matching reviewed procedure; restart never upgrades it.

Protocol authority lives in [Core verification](../services/core/src/payments/providers.rs)
and the [Go payment implementation](../services/extensions/internal/modules/payments/).
Protocol references include [Linux DO Credit](https://credit.linux.do/docs/api),
[Creem checkout](https://docs.creem.io/api-reference/endpoint/create-checkout),
[Waffo Go v2.3.0](https://github.com/waffo-com/waffo-go/tree/v2.3.0) and
[Pancake v0.19.0](https://github.com/waffo-com/waffo-pancake-sdk-go/tree/v0.19.0).
Dependency attribution remains in [THIRD-PARTY-LICENSES.md](../THIRD-PARTY-LICENSES.md).
Source capability and synthetic tests do not establish real merchant or production
acceptance; candidate-specific evidence and remaining work are tracked in [project progress](progress.md).
