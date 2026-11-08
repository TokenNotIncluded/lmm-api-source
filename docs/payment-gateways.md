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

The two sides must agree on channel ID, provider, merchant, environment, currency
and product/store/payment-type binding. A channel ID identifies an immutable policy.
When changing that policy, allocate a new ID and retain the old channel's verification
material for outstanding callbacks. Setting the old Go channel's `enabled = false`
removes it from new checkout selection while allowing recovery.

Core owns `credits_per_minor` and `max_amount_minor`. It freezes the checked integer
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
See [Core runtime configuration](../config/core.example.toml),
[Extensions runtime configuration](../config/extensions.example.toml) and
[database provisioning](../deployment/postgres/README.md). Ordinary startup performs no DDL.

## Refunds and recovery

The native [Stripe](../services/extensions/internal/modules/payments/stripe.go) and
[Waffo Global](../services/extensions/internal/modules/payments/waffo_refunds.go)
adapters support full and partial refund dispatch.
Other native profiles do not expose automatic refunds in their normal configuration.
Unsupported requests are rejected before a refund hold or provider write is created.
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
[Waffo Go v2.1.0](https://github.com/waffo-com/waffo-go/tree/v2.1.0) and
[Pancake v0.17.0](https://github.com/waffo-com/waffo-pancake-sdk-go/tree/v0.17.0).
Dependency attribution remains in [THIRD-PARTY-LICENSES.md](../THIRD-PARTY-LICENSES.md).
Source capability and synthetic tests do not establish real merchant or production
acceptance; candidate-specific evidence and remaining work are tracked in [project progress](progress.md).
