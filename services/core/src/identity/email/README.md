# Email challenge storage slice

Implemented: purpose-bound persistent challenges, target budgets, current
identity/session/authority checks, and transaction-bound consumption. This is
a callable Rust module, with isolated real PostgreSQL tests. No HTTP routes,
mail sender or end-to-end identity workflow are connected.

## Installation boundary

The existing identity contract remains version 5. Neither its canonical
`schema/identity.sql` nor `identity/schema.rs` changes. Core still has 26
components. `email/schema.sql` is a **staged delta**, applied only by the local
test fixture. It is absent from `init-db`, ordinary startup, and upgrade dispatch.
The module constructor validates keys, not installation readiness.

Core fingerprints all non-system catalog definitions. Applying even additive
email DDL to an installed database makes the existing catalog check fail.
Do not apply this delta to a site, change stored fingerprints, advertise
readiness or reuse the Console upgrade procedure to enable it.

Integration requires a separately reviewed identity revision procedure:

1. Freeze the exact predecessor manifest, catalog, source and runtime grants.
2. Reconcile canonical fresh identity DDL with this delta, and explicitly
   advance identity 5 to the next approved version. Do not infer approval from
   this file or this slice's tests.
3. In the owner transaction, protect existing data, apply DDL, update precise
   runtime permissions, and record the complete successor catalog and upgrade
   receipt through the existing reviewed mechanism.
4. Prove fresh/upgrade equivalence, unchanged historical data, restricted
   runtime operation, and startup rejection of old/unmatched contracts.

The delta adds `user_profiles.email_version`, defaulting to zero. It deliberately
does **not** turn existing profile strings into verified authority. It adds
`email_addresses`, `email_challenges`, and target rate buckets/events.
`test_grants.sql` grants only the slice's test needs; it is not the production
grant script. No wildcard or default grants are introduced.

## Composition contract

`EmailAddress::parse` accepts a bare ASCII mailbox, trims spaces, lowercases the
whole address by project policy, and retains dots and plus tags. Controls,
quoted/display addresses and internationalized local parts are rejected.

`EmailChallenges::issue(Request, &EmailAddress)` returns an internal result:

- `Issued` carries the delivery secret once; it has no Debug or serialization.
- `Pending` preserves an existing live code, deadline and attempt count.
- `Suppressed` covers hidden ineligibility and exhausted target issuance budget.

These variants are not an anonymous HTTP response contract. An adapter must
return the same public acceptance envelope and keep secrets inside a durable
encrypted mail enqueue path. No resend payload or outbox exists in this slice;
`Pending` cannot resend the secret and does not claim that mail was sent.

`issue` currently commits its own transaction before returning `Issued`. A
subsequent, separate outbox insert is not a safe integration: a crash between
those commits can leave an active challenge without a recoverable delivery
secret, while later requests only return `Pending`. Before enabling email,
change issuance and encrypted outbox enqueue to commit in the same transaction.
Verify that failures and crashes leave either both records or neither, and that
resending a live challenge retains the original expiry and guessing budget.

`Request::Register` cannot claim an existing authoritative or historical
profile email, including ambiguous legacy strings and username collisions.
`Request::Bind` requires a current personal browser bearer and stable sid,
matching user/session generations, recent primary authentication and, when
required, recent MFA at the current factor revision. It does not itself bind
an email. A new sid cannot reuse another family's challenge.
`Request::PasswordReset` resolves only a verified authority row at the current
email generation, an active user/account, an existing password method and
enabled password policy. Profile email and `email_login_allowed` alone grant
no recovery right. No profile backfill or legacy auto-verification runs.

`verify(...)` owns its transaction:

1. Lock the HMAC target bucket, apply the shared verification budget, then lock
   the expected purpose/email/challenge row.
2. Recheck live authority and database wall-clock time after locks.
3. Commit invalid-code attempts before returning `InvalidProof`.
4. On success, return `EmailMutation` while retaining transaction/authority locks.

The caller writes its identity changes through `mutation.connection()` and
calls `mutation.finish(result_user_id)`. Finish checks expiry again, consumes
once, records a redacted audit and commits all writes together. A bound proof
cannot commit for another user. Drop, business SQL failure, expiry or audit
failure rolls everything back. Do not independently commit the connection.
No independently reusable `verified=true` or detached success token is returned.
Account mutations must use this connection, not a second pool transaction.

The future register/bind/reset callers still own registration policy, unique
authority writes and compatible projection, email-generation changes, password
hash preparation, reset's auth-generation increment, and durable security
notifications. The current tests mutate a profile field to prove atomicity;
they do not claim complete registration, binding or password reset.

## Proofs and limits

Register/bind codes use three random bytes (six lowercase hex); reset uses
sixteen bytes (32 hex). Stored verification is HMAC-SHA256 over length-framed
domain, challenge ID, purpose, canonical address, user/auth/email generations,
sid binding and code, with constant-time MAC comparison. Proof and persistent
target-index keys are separate. Sid and scope use separate MAC domains.
Keys are never stored in SQL. This slice has one configured proof key ID;
missing/changed keys never fall back to plaintext or unkeyed verification.
Key rotation and retained old-key support remain unimplemented.

All processes and purposes share the canonical address's target bucket:

- Issue requests: one per 60 seconds, five per hour, ten per day.
- Verification requests: ten per ten minutes, thirty per hour, including
  purpose mismatches and replays.
- Incorrect codes: five per challenge; the fifth locks that challenge.
- Lifetime: ten minutes, with a final wall-clock check at consumption.

Accepted issuance decisions, including hidden suppression and pending requests,
spend the issue budget. This is a conservative target request limit, not a count
of sent messages. Denied requests do not append unlimited rows or extend windows.
Reopening a store or issuing a different purpose/challenge cannot reset budgets.
This is not a global/IP/user/SMTP limiter. Rate-key rotation, bounded retention
cleanup, encrypted outbox, source limits and mail capacity remain integration work.
Clock and key configuration must remain consistent across Core instances.

## Local verification

`run_selftest.py` compiles the exact leaf sources with explicit already-built
rlibs from the existing locked Core dependency cache; it writes nothing to that
cache. Supply the matching rustc and seven `--extern name=/path/to/file.rlib`
arguments for hmac, sha2, rand, sqlx, tokio, zeroize and subtle, plus `--evidence`.
It uses no Cargo build or network, creates its own socket-only PG18 cluster,
and removes the database and test binary after stopping PostgreSQL.

The evidence contains command/exit status, dependency/source SHA256s, versions,
test log and cleanup result. Tests are also registered under
`identity::email::tests` for a later combined Core build. Database tests are
explicitly ignored outside the fixture runner rather than silently passing.
The runner refuses test URLs outside its own `/tmp/lmm-email-*` fixture.

Self-tests cover canonicalization, persistence/reopen, wrong purpose and sid,
attempt/target limits, unchanged pending deadlines, eight competing stores,
rollback/drop/audit failure, lock-wait/final expiry, restricted runtime writes,
and legacy recovery/generation boundaries.

Complete Core compilation, official upgrade/grants, full recovery credential
revocation, HTTP/Web/SMTP, and independent acceptance remain **NOT RUN**.
