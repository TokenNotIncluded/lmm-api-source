# Payment gateways: fixed Core 28 → 29 upgrade

`core-payment-gateways-v1` is an explicit offline procedure for one exact native
Core predecessor. It replaces the Stripe/USD/fixed-rate intent constraints,
prevents one provider transaction from belonging to two channels, and records
the upgrade. It preserves old business rows, sequence state and permissions.
It does not migrate the old Go monolith, dispatch payments, enable channels or
reopen a site.

The predecessor was built from source
`6523c4b4b831643d69de3874c2e35933c91bfc2b`; its actual PostgreSQL 18.6 UTF-8
catalog was observed separately from the upgrade. The compiled target contains
the complete successor definition list.

| Contract | Predecessor | Successor |
| --- | --- | --- |
| Components | 28 | 29 |
| Tables / sequences | 175 / 22 | 176 / 22 |
| Catalog definitions | 7,035 | 7,068 |
| Manifest stdout SHA-256 | `207bc6bf3f39dc9803177e6cebda491150fb66370a5ff3fc37b26b863cfec97b` | `48578c69632d80d7d4e41a5fb36c67f365875e8bb67e0695f62e1d688177f530` |
| Catalog SHA-256 | `5521fb8224fbdfc994421f0fe04756df99a4e341ceec78b6302882a1f145d2d0` | `cb9349ca60bafbd49204dc237a88eec5ca10ac4654e654e7c816ea59673bc21e` |

The request is the existing `UpgradeRequest` v1 contract, with
`procedure = "core-payment-gateways-v1"`, an original `upgrade_id`, the exact
predecessor and successor manifest bindings, predecessor catalog binding, and
the target database name/OID/system identifier/PostgreSQL version. It also
names distinct installer, owner, runtime and extensions roles. Database
incarnation and role values must come from the intended offline target; the
compiled pins alone are not permission to operate on a database.

The selected installer configuration is passed to
`lmm-core --config /absolute/installer.toml upgrade-db`, with the bounded JSON
request on stdin. The installer must be a restricted LOGIN/NOINHERIT role
allowed to SET ROLE to the NOLOGIN owner. Runtime and extension roles and
application sessions must be offline. The shared engine takes its advisory
fence before the serializable transaction and locks the existing tables.

The operation refuses an unknown predecessor, a changed catalog or existing
provider transaction reuse across channels. It compares old data, sequence
states and privileges before and after the DDL, then requires every successor
catalog definition to match the compiled target. Default grants must not make
`core_meta.payment_gateway_upgrades` readable or writable by runtime,
extensions, PUBLIC or an unrelated role. Runtime checks also reject table or
column reads on this private receipt.

The receipt binds the original typed request, preserved-data hash,
preserved-permission hash and successor catalog. Repeating that exact request
after a confirmed commit returns `already_committed`; a new ID or changed
binding is rejected. A lost acknowledgement is an unknown outcome. Keep the
original request and use the matching recovery procedure; never choose a new
upgrade ID on the assumption that the transaction rolled back.

This command supplies the database-level fence. A real existing-site upgrade
still requires its reviewed backup, service/writer closure, persistent HBA and
history controls. It does not itself establish those controls or authorize
site opening. Earlier 24/25/26/27 component transitions retain their own pins
and preserved installers; this binary is not a universal historical migrator.

## Developer self-test

[`tests/payment_gateways/upgrade_selftest.py`](../tests/payment_gateways/upgrade_selftest.py)
uses a new isolated PostgreSQL fixture and synthetic roles/data. It reconstructs
the fixed predecessor from the old SQL, then invokes the actual candidate CLI.
This distinguishes the observed DDL predecessor from an old executable's
`init-db`. The test checks commit/replay, data and permissions, rejected
conflicts, exact fresh-target catalog, and runtime receipt restrictions.
The verification record reports what was actually executed. Synthetic tests
are separate from independent acceptance, lost-ACK testing and production
upgrade evidence.
