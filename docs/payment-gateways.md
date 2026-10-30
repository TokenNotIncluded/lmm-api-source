# Native wallet payment gateways

The native wallet supports six provider protocols: Stripe, EPay, Creem, Waffo
Global, Waffo Pancake and Coinbase Business Checkout. EPay also supplies the
Alipay, WeChat Pay and Linux DO Credit channel presets. The first five protocols
restore the seven wallet choices in the
[legacy template list](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/web/src/features/system-settings/integrations/payment-method-templates.ts).
FastPay is retired and is not an accepted provider or a wallet choice.

This implementation covers native wallet top-ups. Provider subscription
lifecycle, external merchant-store checkout, the former Pancake administration
screens and all-provider refunds remain separate work. The
[recipe parity matrix](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)
retains those requirements. Code and synthetic self-tests do not establish
real merchant integration or production acceptance.

## Supported top-up profiles

| Protocol / wallet channel | Settlement unit | Checkout and proof admission |
| --- | --- | --- |
| Stripe | USD cents | Hosted Checkout and the existing Core HMAC verification. Existing direct-account refund handling remains available when separately authorized. |
| EPay Alipay / WeChat Pay | CNY fen | Core signs a fixed intent; Go receives no MD5 merchant key. GET or form callbacks are independently verified in Core. |
| Linux DO Credit over EPay | Hundredths of LDC | Core signs the `epay` payment type for `https://credit.linux.do/epay/pay/submit.php`. Checkout requires an explicit form POST. LDC has its own conversion policy and is reported as a virtual unit, separate from cash currencies. |
| Creem | USD / CNY / EUR minor units | One configured, active, one-time product at a fixed amount. Checkout checks the actual product price, currency, environment and inclusive tax before creating a session. Core verifies the original HMAC callback and its product/order/amount binding. |
| Waffo Global | USD cents | Hosted checkout through the official order protocol. Core verifies the RSA callback and independently queries the frozen order with its own configured provider credentials before accepting new proof. |
| Waffo Pancake | USD cents / CNY fen | Authenticated checkout using the official SDK. Core verifies the timestamped RSA callback, environment, merchant, store, product, original order and actual paid amount. |
| Coinbase Business Checkout | USDC, quoted as USD cents at 1:1 | Hosted checkout with a CDP-signed API request. Core verifies the original webhook signature and admits only a completed payment bound to the frozen order, merchant, environment and exact amount. |

Only the existing Stripe adapter exposes automatic refund dispatch. An unsupported
refund returns an error before creating a hold or provider request. A provider
API read is an observation; it cannot replace independently accepted proof or a
Core ledger receipt. This also means EPay's callback path is available while
its Go-side credentialed lookup is deliberately unavailable: that lookup would
require the Core-only merchant key.

## Configuration and ownership

Use [core-payments.example.toml](../config/core-payments.example.toml) for Core's
verification and conversion policy, and
[extensions-payments.example.toml](../config/extensions-payments.example.toml)
for Go's outgoing adapters. Both documents use `version = 2` and an explicit
provider and environment on each channel. Version 1 Stripe test/live
configuration remains accepted. The existing isolated Stripe test profile
remains a separate laboratory configuration.

Each side must agree on channel ID, provider, merchant, environment, currency
and product/store/payment-type binding. A channel ID represents an immutable
payment policy. When changing the merchant, product, currency or conversion,
allocate a new channel ID and keep the old channel configured for outstanding
callbacks. Set the old Go channel's `enabled = false` to remove it from new
checkout selection while retaining callback recovery. Do not reuse an old ID
with a new merchant's credentials.

Core owns `credits_per_minor` and `max_amount_minor`. The exact quote is
`amount_minor × credits_per_minor`, with checked integer bounds, and is stored
on the immutable intent. The ledger unit stays `credit_500k_usd`. Stripe USD
uses the existing 5,000 credit units per cent. Non-USD rates in the example are
illustrative administrator policy; they are not live FX or pricing defaults.

`webhook_secret_file` is the historical field name for the Core verification
material. It contains a Stripe/Creem/Coinbase webhook secret, an EPay merchant key or a
Waffo provider public key, depending on the protocol. Waffo Global additionally
requires `inquiry_api_key_file` and `inquiry_private_key_file` in Core for both
test and live environments. Core makes this inquiry itself against the fixed
official endpoint; callers cannot supply a URL or an alleged query response.
The inquiry runs before the database transaction, and the intent is rechecked
inside the transaction before proof is persisted.

Go holds only the credentials needed for outgoing provider operations. EPay
has no Go API-key or merchant-key setting: `GetEPayCheckout` returns the
checkout method and signed URL for an already frozen intent. Standard CNY EPay
uses GET; Linux DO Credit uses POST with the URL query fields as form inputs.
The Linux DO callback may omit `sign_type`; when present it must be `MD5`.
Standard CNY EPay callbacks require `sign_type=MD5`. Core cannot sign arbitrary
caller fields. Core verification keys, original callback signatures, provider API
keys and checkout bearer URLs must not appear in public status or logs.

Secrets are bounded private regular files, owned by the appropriate service
or root, with mode `0400` or `0600`. The Go provider document itself is private.
Never copy an actual secret into a TOML example or browser configuration.

To assemble the feature, provision the separate payment database and its
restricted Go role, set `host.modules` to include `payments`, enable
`host.web_compat`, and supply `[core].payment_token_file` plus `[payments]`
storage and configuration settings. Enable `callbacks_enabled` only with the
callback ingress in place. Core needs its own payment configuration, narrow RPC
permissions and payment runtime grants. Normal startup performs no DDL.
See [runtime configuration](../config/extensions.example.toml) and
[database provisioning](../deployment/postgres/README.md).

## Coinbase Business Checkout

Coinbase receives USDC and the native wallet quotes the same numeric USD amount.
For example, a quote for 1,000 USD cents creates a checkout for `10.00 USDC`.
This is the top-up's explicit conversion policy, using integer cents; it is not
a live exchange-rate lookup. The payment form and confirmation display USDC
before the customer opens Coinbase. Core continues to own the exact credit
quote and the eventual ledger entry.

Use an ECDSA CDP API key for the Coinbase Business account. Go's `api_key_file`
contains the CDP key name, and `private_key_file` contains its P-256 PEM private
key. Go signs a short-lived JWT for each outgoing request. Core holds the
webhook subscription secret independently in `webhook_secret_file`; Go never
loads that secret. The configured `merchant` is this installation's stable
account label, and must match on both sides.

Create separate channel IDs and webhook subscriptions for test and live use.
Test channels use `https://business.coinbase.com/sandbox/api/v1/checkouts`;
live channels use `https://business.coinbase.com/api/v1/checkouts`. The sandbox
accepts the same CDP API credentials, but its webhook subscription must include
the label `sandbox = "true"`. Each channel needs its own subscription secret
and callback URL, such as `/callbacks/8`. Coinbase does not document a signed
payload field that independently identifies sandbox mode, so do not reuse a
subscription secret across test and live channels.

Subscribe to `checkout.payment.success`. Only the signed event with
`status = "COMPLETED"`, `currency = "USDC"` and the exact frozen amount can
become payment proof. Coinbase's webhook `id` is the checkout ID; the local
event identity also includes the event type. `X-Webhook-Signature` is the current
signature header, and Coinbase also sends the deprecated `X-Hook0-Signature`.
Matching dual headers are accepted; conflicting values or repeated headers are
rejected. Payment returns and API status observations cannot credit a wallet.

The provider accepts decimal amounts from `0.01` to `100000000`, with at most
two fractional digits. The configured Core maximum can impose a lower limit.
Checkout requests retain one UUIDv4 idempotency key for the original operation.
Coinbase does not publish an idempotency retention duration in this API
contract, so an ambiguous write is retained for reconciliation instead of
automatically repeated after an assumed safe interval. Coinbase refunds and
subscriptions are outside this wallet top-up adapter.

This integration uses the current Business Checkouts API. The retired Coinbase
Commerce Charges API is not used. Setup and protocol references:
[CDP API authentication](https://docs.cdp.coinbase.com/coinbase-business/authentication-authorization/api-key-authentication),
[create checkout](https://docs.cdp.coinbase.com/api-reference/business-api/rest-api/checkouts/create-checkout),
[webhook events](https://docs.cdp.coinbase.com/coinbase-business/checkout-apis/webhooks),
[signature verification](https://docs.cdp.coinbase.com/webhooks/verify-signatures)
and [sandbox setup](https://docs.cdp.coinbase.com/coinbase-business/checkout-apis/sandbox).

## Browser quote and confirmation

The native wallet uses a personal Core Session. Core derives the personal
economic account; the browser cannot supply an account ID, conversion rate or
credited amount. The server exposes only configured, enabled channels.

| HTTP route | Purpose |
| --- | --- |
| `GET /api/user/payments/channels` | Read available channel IDs, providers, units and any fixed Creem amount. |
| `POST /api/user/payments/quote` | Prepare an immutable Core intent from channel, currency, amount and original request key; no provider checkout or credit is created. |
| `POST /api/user/payments/checkout` | Confirm the original order ID and exact quoted credit amount, then queue the provider checkout. |
| `GET /api/user/payments/checkout/{top_id}` | Read the authenticated owner's checkout link and the exact order's current state. |
| `GET /api/user/payments/orders/{top_id}` | Retain the older minimal status-only contract, without a checkout URL. |

IDs and monetary values in the new browser contract are canonical decimal
strings. The UI uses integer arithmetic and distinguishes the provider's paid
state from Core's credited state. It displays the exact settlement currency and
credit quote before confirmation; the gateway page is opened only on a user
click. `checkout_method` explicitly selects a GET link or a POST form; the
browser does not infer a method from the URL. A return-page URL is never
evidence of payment.

The all-site financial projection includes only native payments with a Core
credit journal. It reports USD, CNY and EUR independently as cash currencies,
and LDC under virtual units. Neither aggregation nor display converts LDC to
USD or combines unlike settlement units.

The browser saves the request and its original key before the first write. A
timeout, lost response or reauthentication does not invent another operation.
Writes disable automatic authentication-refresh replay. The user can recover
or explicitly retry the saved operation, while status polling uses bounded GET
requests. Only the matching Core-confirmed receipt can display completion.

The edge must route the whole `/api/user/payments/` prefix to Extensions ahead
of Core's `/api/user/` prefix, retaining the user's Bearer header. Callbacks
instead use `/callbacks/{numeric_channel_id}`, accept provider-specific GET or
POST, retain the exact original query/body/signature and remove browser/service
credentials. Both families require appropriate request limits and no sensitive
query/body logging. The repository's local edge harness checks the candidate
route contract; it does not update a public TLS server block. See
[edge compatibility](edge-cutover-compatibility.md).

## Database transition

Fresh installation retains component 29, `payment-gateways`. The component
keeps the original `payments.sql` bytes unchanged, replaces the Stripe-only
provider/currency/rate constraints, adds unique provider transaction ownership
across channels, and creates an immutable, owner-only upgrade receipt. It does
not convert balances, reprice existing intents or import historical payments.

An existing native Core database uses the explicit fixed procedure
`core-payment-gateways-v1` through `upgrade-db`; see
[the exact 28 → 29 upgrade contract](payment-gateways-upgrade.md). Updating
source or restarting services does not upgrade a database. A different
predecessor needs its matching reviewed procedure.

Coinbase adds component 32, `coinbase-payments`, without editing previous schema
components. Existing databases at the exact 31-component contract use the
explicit `core-coinbase-payments-v1` procedure. The
[Coinbase schema upgrade guide](schema-coinbase-payments-upgrade.md) records its
predecessor, fingerprints and offline steps. The procedure expands the provider
constraint; it does not alter balances or enable a channel.

## Implementation references and validation limits

The protocol implementations are in
[`payments/providers.rs`](../services/core/src/payments/providers.rs) and the
[Go payment module](../services/extensions/internal/modules/payments/README.md).
The fixed product and callback formats follow the legacy source plus the
[Linux DO Credit API](https://credit.linux.do/docs/api),
[Creem checkout API](https://docs.creem.io/api-reference/endpoint/create-checkout),
[official Waffo Go v2.1.0 SDK](https://github.com/waffo-com/waffo-go/tree/v2.1.0)
and [official Pancake v0.17.0 SDK](https://github.com/waffo-com/waffo-pancake-sdk-go/tree/v0.17.0).
Pinned dependency attribution is retained in
[THIRD-PARTY-LICENSES.md](../THIRD-PARTY-LICENSES.md).

The tests exercise original signed bytes, wrong amount/merchant/order rejection,
duplicate proof and credit handling, native quote/confirmation and isolated
PostgreSQL upgrades. Executed results are recorded in the
[payment verification record](../services/extensions/internal/modules/payments/VERIFICATION.md).
Unrun real-provider, browser, process-restart and refund scenarios remain
unverified; the older Stripe acceptance record does not certify the new gateways.
