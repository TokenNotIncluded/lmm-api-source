# Current integration verification

This delivery extends source base 6ae84b4e5ab65120c5542ae9d2ed2921a9ba2da8
with the preserved channel projection work and the current feature candidates.
It is a local development delivery, not deployment or complete business acceptance.

## Implemented and locally checked

- Component 44 adds channel usage projection and explicit historical openings.
  Shared usage reads stream beyond the former implicit row limits.
- Component 45 adds trust-policy preview, independent owner-enrolled WebAuthn
  approval, atomic confirmation, outcome resolution and immutable audit.
  Ordinary runtime database authority cannot enroll an editor or read its
  signing key. The optional online feature is disabled by default.
- The owner CLI provisions or rotates its separate signing key with an expected
  fingerprint. Unknown request fields are rejected, including status requests.
  Fresh first-owner initialization recognizes only the exact current seed
  contracts and refuses preprovisioned editor authority.
- Explicit audio rules support non-streaming native-key Chat Completions with
  separate text/audio input/output counters, exact pricing, reservation,
  persisted terminal usage, settlement and recovery using the original price.
  Incomplete usage retains the hold. Other audio entries remain unfinished.
- Legacy provider balance mappings preserve manual configuration precedence,
  explicit units, unknown values and the configured provider origin.
- The trust-policy Web flow uses actual preview values, explicit confirmation,
  WebAuthn and same-token outcome recovery. Unavailable editor authority is
  read-only. The existing quoted-wallet behavior remains available.

## Local evidence

Checks used Rust 1.99.0, Bun 1.3.14 and isolated PostgreSQL 18 with synthetic
records. Provider tests use synthetic peers; no paid provider call was made.

| Check | Result |
| --- | --- |
| Complete Core library run | 1274 passed, 3 failed, 19 ignored |
| Follow-up index contracts | 12 passed, including the preserved historical 26-component catalog |
| Follow-up editor configuration and legacy-key tests | Passed; the other two library failures were fixed |
| Owner binary tests | 10 passed initially; the closed status-request parser failure was fixed and its targeted test passed |
| Five offline upgrade procedures | Six integration cases passed: completion, account appeals, channel accounting, channel projection and trust-policy editing |
| Complete Web serial suite | 4912 Bun tests and 47 Node tests passed; 715 source files and 7 additional script files |
| Web typecheck | Passed after increasing the isolated check's memory allowance; the first resource timeout remains recorded |
| Production Web build and Core executable build | Passed |
| Historical channel accounting Python tests | 26 passed |
| Fresh 45-component installation | Passed; its measured catalog equals the separate 44-to-45 additive construction |

The original Core failures and their follow-ups are separate evidence; the
complete library suite was not rerun after those targeted fixes. Ignored email,
payment and benchmark cases are not counted as passed. The trust service's
23 tests, audio module's 10 tests, charging module's 73 tests and balance
control's 7 tests passed within the complete library run.

The separate 44-to-45 procedure is core-trust-policy-edit-v1. It preserves
existing policy, financial rows and pending projection work. Earlier procedures
retain their frozen 44-component targets.

- Predecessor manifest: 5f6b75aaae3110c460c58270aac77028d8d4c32360481a52831872515131261d
- Target manifest: 567d84fa06adf6916a62b6a5de0ae4234da1c0876972254f8393f0a55cd8d9c8
- Target catalog: b3e3db14753bcb575eba90541b09cf9fac2955a22e513741a91b1f11019dbe6f
- Checked Core executable: 91b6f28e56828cff9d3585e6a5e4ab64a8dc2fd8e8be31adb4e3af50dde1eb3f

The historical catalog fixture and extraction evidence are in
[index26_a7e1105d3.md](../services/core/tests/core_schema/index26_a7e1105d3.md).
The new catalog does not replace that historical approval.

## Offline registration follow-up

The Web now accepts only the complete public owner-CLI begin response, checks
the RP and browser origin, creates a user-verified WebAuthn credential and
exports its public response. Creation does not approve an editor. Owner
database access, key provisioning, review and fingerprint approval stay offline.
All seven locales include the registration flow.

On the Core executable identified above, a new isolated 45-component database
completed actual init-db, create-first-owner, initial signing-key provisioning,
CLI begin, Chromium credential creation, CLI review and explicit approval.
No bootstrap fallback was used. Nine browser/CLI cases passed, including wrong
origin, cancellation, public-only export and pending/reviewed/approved states.
The actual CLI also rejected the extra-field status-request counterexample.
The browser used a CTAP2 virtual authenticator and the real staged component
with minimal fixture CSS; this is not full-app routing or theme acceptance.

The integrated Web passed 28 focused tests, scoped lint/format, typecheck and
a fresh production build. Its initial gzip bundle is 558670 bytes against the
786432-byte limit. All 2742 frozen Web inputs remained unchanged through these
checks. Older binary and selector failures remain separate evidence.

## Still incomplete

Streaming and other audio entry points, the full provider/format matrix,
historical channel sorting and freshness, final identity import-contract
review, complete financial capability parity, and final resource/rollback
acceptance remain open. The importer candidate is intentionally held for the
final schema review; source-fingerprint rejection still blocks its dependent
trust-import suite. No fingerprint was changed merely to make that suite pass.

Real Core/Go browser business acceptance, full-app registration presentation,
final million-record projection evidence, independent whole-business sign-off,
and production deployment are not complete. No production user, account,
business database or running service was changed by these local checks.
