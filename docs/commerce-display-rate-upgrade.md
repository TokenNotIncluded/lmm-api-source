# Funded self-purchases and display exchange rates

The `commerce-self-purchase` and `display-exchange-rate` components extend the
Core database from 56 to 58 components. The first supports a seller buying their
own product through the existing reserve, delivery, settlement and refund
operations. The second stores the CNY-per-USD display preference used by the
settings page and public status response. The compiled definitions are in
[commerce_self_purchase.sql](../services/core/schema/commerce_self_purchase.sql)
and [display_exchange_rate.sql](../services/core/schema/display_exchange_rate.sql).

## Product behavior

A self-purchase must fund the entire order before delivery. At the current zero
store fee, capture moves the reserved amount back to the same economic account
using the existing debt-repayment rules. A full refund of that capture records
the original amount and transaction identity without another balance change.
Generic self-transfers remain invalid. The zero-entry refund exception applies
only to the original, full `core-commerce` self-capture; unrelated journals keep
their existing balance and entry constraints.

Extensions checks the negotiated `commerce.self-purchase.v1` capability before
submitting a self-purchase. An older Core cannot gain support merely because its
ordinary commerce endpoint is reachable. Product visibility, stock, terms,
purchase limits and current account authorization remain part of checkout.

`USDExchangeRate` is a display preference. A superadministrator session can save
it through the existing option endpoints, including the existing expected-value
comparison and audit transaction. The only new database setter is
`core_console.write_display_exchange_rate(text)`. It accepts a finite numeric
value from `0.0001` to `1000000000`; it cannot change another option. The
`/api/option/exchange-rate?currency=CNY` helper reads a reference quote and returns
its provider date separately from the fetch time. The user must save a value for
it to become the display preference. Ledger credits and payment quotations keep
their existing units and sources.

## Exact installation contract

Fresh `init-db` installations include both components. Existing component-56
databases use procedure `core-commerce-display-rate-v1` through the offline
`upgrade-db` interface. The earlier 56 component definitions remain unchanged.
Earlier upgrade procedures retain their own frozen successors; this procedure
does not bridge an arbitrary older database directly to component 58.

| Binding | SHA-256 |
| --- | --- |
| Predecessor manifest | `f2340fb850e81f168ababe0301d80bd85769b235a8c5b5abb91f0bcd7429624e` |
| Predecessor catalog | `46cbd8e0a9e9bb6c55cb899558b40fa8c8df1affa2e366cbbde733a8527bda43` |
| Successor manifest | `02a013915ad4b77dd4cd4006ff62c07653e474afe690b6927892c10168ab400a` |
| Successor catalog | `e4ca2e9475c7513fbc551dcc78fdfed2f5f792ce98961620c329dbf74836b55b` |

These values are checked by the
[procedure](../services/core/src/core_schema/upgrade/commerce_display_rate.rs)
and its [frozen target](../services/core/src/core_schema/upgrade/commerce_display_rate_target.json).
The request binds `version: 1`, the procedure, one stable `upgrade_id`, the actual
database name/OID/PostgreSQL-18 version/system identifier, the distinct
installer/owner/runtime/extensions roles, both predecessor hashes and the
successor manifest hash. Those database and role fields must come from the
installation being upgraded.

Once its writers are fenced and drained, the operator invokes the existing
offline installer with that bound request:

```sh
lmm-core --config /absolute/installer.toml upgrade-db < /absolute/core56-to-core58.json
```

The transaction checks the exact predecessor and preserves prior rows, sequence
state, ownership and existing grants. It replaces only the two declared ledger
functions and the self-purchase constraints, adds the display setter, and grants
runtime only EXECUTE on that new setter. The immutable receipt is stored in
`core_meta.commerce_display_rate_upgrades`. Runtime has no receipt write access.
The [shared installer preflight](../services/core/src/core_schema/upgrade.rs)
checks database identity, role separation and drained writers before apply.

An uncertain commit is resolved using the original request and `upgrade_id`
under the same fence. An exact committed receipt returns `already_committed`;
a mismatched receipt, catalog or database identity is rejected. Ordinary startup
only checks the installed contract and runtime authority. It does not execute
this upgrade or alter grants.

## Verification scope

The Rust integration targets are `commerce`, `commerce_display_rate_upgrade`
and `display_exchange_rate`. The SQL fixtures exercise the same compiled schema
inputs with separate disposable PostgreSQL identities. SQL verification does
not establish that the Rust HTTP or offline installer process has executed.
Current commands, results, failures and independent evidence are recorded only
in [the shared progress list](progress.md), under STORE-03, APP-09 and CHECK-07.
