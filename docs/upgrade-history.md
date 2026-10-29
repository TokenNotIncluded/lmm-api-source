# Independent upgrade history

`lmm-core upgrade-history` installs or inspects an external PostgreSQL 18
history authority. It does not execute a target migration, certify a writer
fence, issue a retry, release a slot or reopen a site. Core startup never
installs this schema. The full upgrade procedure must combine this history
with actual deployment fencing, target receipts and reconciliation.

Use a separately administered PostgreSQL cluster outside the target's restore
and failover domain. The client rejects the target's physical system identifier,
a wrong authority ID, a replica, disabled fsync/full-page writes, and a writer
with owner membership or direct mutation privileges. Two test postmasters on
one laptop prove separate database state, not production fault-domain isolation.
Restoring the authority itself to an earlier state violates this deployment
contract; this store cannot detect an administrator rolling back its own trusted
server and identity. Do not fail over it to a stale replica.

The authority owner is NOLOGIN. The offline installer may SET ROLE to that
owner; the maintenance writer is a separate unprivileged LOGIN without owner
membership. It receives only schema USAGE and four fixed-search-path functions.
Application runtime accounts must have neither authority credentials nor these
grants. The owner needs EXECUTE on `pg_catalog.pg_control_system()` for live
physical identity checks. Owner, server and deployment administrators are trusted.

Supply an explicit private authority configuration using the existing Core TOML
format, with `version=1` and `[database].url_file` pointing to a private DSN file.
Use a verified TLS connection for a remote authority. Installation accepts:

```json
{"action":"install","authority_id":"<64 lowercase hex digits>","owner":"history_owner","writer":"history_writer"}
```

Run it with the installer configuration:

```sh
lmm-core --config /private/history-installer.toml upgrade-history < install.json
```

Installation is one transaction in an otherwise empty authority database.
Existing authority state is never recreated, erased or silently initialized.
Inspection uses the writer configuration and `action=inspect`, `authority_id`
and the exact original `binding`. The binding contains `site_id`, `operation_id`,
request/candidate SHA256, target system identifier, database and OID. Inspection
reports observed history only; `site_open_allowed` and `retry_allowed` remain false.

The Rust client reserves an original operation, records a separately verified
fence, consumes it once, then records a separately verified commit receipt.
These produce closing/held/started/committed at sequences 1/2/3/4. PostgreSQL row
locks and compare-and-set serialize competing consumers. Repeating a reservation
only reads the same bound operation. A different request or operation conflicts;
there is no expiration, automatic takeover or new generation after commit.
The controller must validate supplied fence/receipt facts: the history store
records observations and does not independently inspect the target or host.

A lost mutation COMMIT acknowledgement returns `UPGRADE_HISTORY_COMMIT_UNKNOWN`.
Read the original operation through a fresh authenticated connection. Missing
history never proves NOT_COMMITTED; a started operation cannot be retried.
Committed history and a restored target missing its receipt mean divergence,
not permission to execute again. The full resolver and finalization procedure
must enforce those decisions; this management command does not replace them.

Local developer checks use `scripts/test-upgrade-history.py` with explicit
Core/test executables and a new evidence directory outside the checkout.
The runner creates two private PostgreSQL clusters, physically restores only
the target, checks exact CAS consumption and role boundaries, and removes
successful fixtures. Failed inputs and logs are retained. This is development
evidence, separate from independent acceptance and production cutover.

PostgreSQL references: [row locking](https://www.postgresql.org/docs/18/explicit-locking.html),
[database admission](https://www.postgresql.org/docs/18/sql-alterdatabase.html).
