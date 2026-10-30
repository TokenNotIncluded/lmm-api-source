# Coinbase payment schema upgrade

Coinbase adds one allowed provider name to existing payment intents. The same
Core-owned intents, accepted proofs, transaction ownership constraint and ledger
continue to determine whether an account receives credit.

## Installation contract

The new `coinbase-payments` component is appended after the existing 31
components. It changes `core_payments.intents_provider_check` to include
`coinbase` and creates the owner-only, immutable
`core_meta.coinbase_payment_upgrades` receipt table. Historical component SQL
bytes and their fingerprints remain unchanged. No application rows, balances,
grants, provider payloads or credentials are imported.

Fresh installation runs the compiled 32-component schema through `init-db`.
Existing installations use the explicit `core-coinbase-payments-v1` procedure
through `upgrade-db`. Ordinary startup checks the installed contract; it does
not execute this upgrade.

The procedure accepts only the 31-component predecessor at source commit
`b521f5575bebe4de7195bbd420b16637f46042d5`. Its fixed fingerprints were observed
on an isolated PostgreSQL 18.6 database with UTF-8 encoding:

| Contract | SHA-256 |
| --- | --- |
| Predecessor schema manifest | `6991ca03dbef6ea9ec788bf191d022f8e6a45f5f7ecbb1190255e5dd9c1b5735` |
| Predecessor catalog | `68cedab606bceb54212a78fb595b65755188ee17f8155e3a9588e554548e482e` |
| Successor schema manifest | `9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15` |
| Successor catalog | `abe3979b376da869d4a05b32dda8c3ad33197497db7af16970d8c9b1153bc6ce` |

The manifest hashes use compact JSON plus its final newline. Catalog hashes use
the compiled `CATALOG` query and length-prefixed UTF-8 definitions, exactly as
Core startup checks do. Older installations first need their corresponding
preserved installer and reviewed upgrade sequence, including the 29-to-31
request-operations step. The current binary rejects historical procedures whose
compiled successor no longer matches.

## Explicit offline apply

Use the existing offline process from
[the schema guide](../services/core/schema/README.md): stop every writer,
verify the backup and target database identity, and prepare separate installer,
owner, runtime and extensions roles. The owner remains `NOLOGIN`; the dedicated
installer logs in and selects that owner. Runtime and extensions credentials
are not installation credentials.

Create a reviewed JSON request with the following fields. Replace the sample
database identity and role names with observations from the intended isolated
or maintenance target. Those fields are checked against the live connection.

```json
{
  "version": 1,
  "procedure": "core-coinbase-payments-v1",
  "upgrade_id": "coinbase-upgrade-reviewed-001",
  "database": {
    "name": "REPLACE_DATABASE_NAME",
    "oid": 12345,
    "system_identifier": "1234567890123456789",
    "server_version_num": 180006
  },
  "roles": {
    "installer": "REPLACE_INSTALLER",
    "owner": "REPLACE_OWNER",
    "runtime": "REPLACE_RUNTIME",
    "extensions": "REPLACE_EXTENSIONS"
  },
  "predecessor_manifest_sha256": "6991ca03dbef6ea9ec788bf191d022f8e6a45f5f7ecbb1190255e5dd9c1b5735",
  "predecessor_catalog_sha256": "68cedab606bceb54212a78fb595b65755188ee17f8155e3a9588e554548e482e",
  "successor_manifest_sha256": "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15"
}
```

The invocation consumes that request on standard input:

```sh
lmm-core --config /absolute/private/installer.toml upgrade-db \
  < /absolute/private/coinbase-upgrade-request.json
```

The existing offline guard binds the database system identifier, database OID,
roles, installer session and backend lifetime before any DDL. Under its table
locks, this procedure verifies the predecessor catalog and component receipts,
captures existing rows, sequence state and permissions, applies the one
component, and requires the fixed successor catalog. It compares the captured
data and permissions before updating schema receipts. Any mismatch rolls back
the transaction.

If COMMIT has an unknown outcome, reuse the exact original request and upgrade
ID. A matching durable receipt produces `already_committed`; a different
request, a fresh installation without the original receipt, or changed catalog
is rejected. No new request key is generated for recovery.

## Verification status

The component SQL was applied to a fresh isolated 31-component schema and to a
fresh 32-component installation on PostgreSQL 18.6. The upgraded catalog matched
the fresh successor exactly: only the provider constraint changed and the
upgrade-receipt definitions were added. Rust source includes targeted signature,
environment binding, policy and upgrade-request tests, plus Coinbase in the
existing real-PostgreSQL proof-to-credit test.

The complete Core build and Rust test run were blocked in this environment by
the unavailable pinned `oxide-auth` Git dependency. Consequently the Rust
offline command, permission-preservation checks and full credit transaction
have not been executed here. Run those source tests against an isolated database
with the locked dependency set before accepting an existing-site upgrade.

Configure the channel and distinct sandbox/live verification secrets using
[the payment guide](payment-gateways.md) after the schema contract is accepted.
The connection and webhook formats follow Coinbase's
[Checkout webhooks](https://docs.cdp.coinbase.com/coinbase-business/checkout-apis/webhooks)
and [signature verification](https://docs.cdp.coinbase.com/webhooks/verify-signatures).
