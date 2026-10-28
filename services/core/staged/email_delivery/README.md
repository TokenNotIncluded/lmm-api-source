# Staged email delivery outbox

This is a callable development slice. It is not imported by Core, started at
startup, exposed through HTTP, or advertised as ready. Its local delta is not
part of the identity component contract or the production grants.

## Atomic issuance

`Outbox::issue` owns one PostgreSQL transaction. `Outbox::issue_in` composes
with an existing caller transaction. Both invoke
`EmailChallenges::issue_in` on that transaction, which locks and records the
persistent target budget and creates/reuses the challenge without committing.
The outbox payload is then AEAD-encrypted and inserted before the transaction
commits. A challenge insert, budget write, or outbox insert failure therefore
cannot leave a newly issued code without its durable payload. `Outbox::issue`
returns no challenge ID, code, attempt status or eligibility decision.

The old `EmailChallenges::issue` API remains available and commits by itself.
Its result must never be passed into a separate outbox transaction. The new
composition entrypoint is the only issuance path in this slice.

For a pre-existing live challenge with no recoverable encrypted payload, the
outbox does not send, claim mail was sent, rotate the proof or extend its
expiry. It returns the same unit result as hidden ineligibility. If an
unexpired prior payload exists and no attempt is pending or leased, a new
explicit issuance request can create a new SMTP attempt using the same secret
and original deadline. The shared persistent target budget applies. Unknown
attempts are not automatically resent.

`EmailChallenges::verify_current` accepts the trusted purpose, canonical email,
current authenticated subject where required, and code. It computes the
existing scope key inside Core and locks the matching active challenge. The
client does not need a challenge ID. It commits target verification budget
before resolving the subject or looking up a challenge, including a miss. A
miss and a wrong code return the same `InvalidProof`; a wrong code also commits
the challenge attempt count. The existing ID-taking `verify` remains for
compatibility.

## Worker and SMTP facts

Workers claim one row using `FOR UPDATE SKIP LOCKED`, a random owner token and
a database-clock lease. They release database locks before SMTP I/O. A second
database transaction rechecks the lease, challenge deadline, subject
generations, verified recovery authority, and for bind mail the current session,
recent primary authentication and required current-revision MFA. Only then is
`smtp_started_at` set and the connection opened.

Final updates compare attempt ID, owner token and a still-live lease. An expired
owner cannot overwrite another state. The reaper marks an expired lease
`unknown`; it never makes the same attempt pending again. A new attempt requires
an explicit issuance call and uses a new Message-ID. Thus the SMTP window after
DATA can still produce a duplicate after an uncertain ACK; exactly-once delivery
is not claimed.

`accepted_by_smtp` means `lettre` received the final positive SMTP response
(250). A 4xx/5xx SMTP response is `rejected`; a timeout, connection loss,
malformed response or lost final response is `unknown`. Provider diagnostics,
recipient, message and credentials are not formatted into errors or logs.
Challenge and notice bodies are fixed templates. No password is a payload
field. Challenge ciphertext is erased on challenge invalidation/consumption or
expiry; notice ciphertext expires within 24 hours.

SMTP TLS is required except for the explicit test-only mode, which accepts only
a numeric loopback IP and no authentication. Credential files must be absolute,
non-symlink regular files with Unix permissions 0400 or 0600. The outbox AEAD
key is an independently generated 32-byte raw file supplied through
`MailKeys::from_files`; key rotation retains old key IDs until their ciphertext
is erased. Do not reuse proof, MFA, session or HumanGrant keys.

## Explicit limits and integration blocker

- The slice does not implement resend endpoints, global/IP SMTP capacity
  controls, cleanup scheduling, formal schema versioning, production grants,
  readiness, HTTP routes or application worker startup.
- Callers that change an account and enqueue a security notice must perform
  both operations in one transaction. They must not call a notice API after
  committing the account mutation.
- `EmailChallenges::issue` remains a standalone committed API for existing
  callers. Until all product adapters are moved to `Outbox::issue` or a shared
  transaction, the product has no atomic durable-mail issuance contract.
- A locally successful SMTP-sink test is not Core integration, product
  acceptance, production SMTP evidence or a promise of inbox delivery.

## Local check

Run `run_selftest.py --help`. The runner compiles the unchanged locked leaf
dependencies already present in the named target directory, starts a private
socket-only PostgreSQL cluster, and runs both the existing challenge tests and
these outbox tests against the same synthetic fixture. It does not use Cargo,
CI, network access or real SMTP. SMTP acceptance tests bind only to
`127.0.0.1`. Logs must never contain plaintext addresses, codes, links,
passwords or credential values.
