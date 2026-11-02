# Current integration verification

## Channel observations and audio Chat streams

The current change extends source 2842ce19de058338f5b87628001753dd2c65cdb1.
Component 46 stores model-directory observations separately from money and
routing publication. Native channel statistics compare exact consumption,
explicit balance units and scoped probe times before pagination. Unknowns stay
last; changed data rejects the previous page digest. Cached pages are validated
again, including an A-to-B-to-A navigation within the normal ten-second cache.

Chat audio streams retain the four text/audio counters, frozen exact pricing
and pending holds when final usage is unavailable. The worker reads through
actual HTTP EOF and atomically records terminal evidence and settlement usage.
Explicit OpenAiChatV2 supports omitted text counters only within its restricted
text/audio contract and with explicit audio counters. Normal obfuscation and
service_tier metadata is accepted without persisting response content. New
journals retain raw usage and frame hashes; recovery uses the original format
and price. V1 valid explicit-counter receipts retain their pricing semantics;
duplicate JSON keys are now rejected, including in V1.

| Current scoped check | Result |
| --- | --- |
| Core observation, sorting, HTTP, permissions, first-owner and new upgrade targets | 23 passed |
| Five older upgrade procedures | Six cases have passing follow-ups; initial completion and trust-policy failures remain recorded |
| Chat audio module, including PostgreSQL and synthetic streaming peers | 27 passed |
| Charging regression on the Chat V2 test executable | 73 passed |
| Web statistics, cache navigation and existing management regressions | 30 passed |
| Web typecheck, production build and initial bundle budget | Passed; 558663 gzip bytes against 786432 |
| Independent component46 CLI and actual role scripts | 58 commands matched expectations, including 16 expected refusals |
| Independent audio v3.1 source review | No scoped blocker; not independent process-level business acceptance |

The initial combined upgrade run failed two exact-rejection assertions. Each
subsequently passed with diagnostics; the first failures' actual rejection
causes remain UNKNOWN. The completion fixture now observes its own database's
session exit at six explicitly offline boundaries. It never retries an upgrade,
terminates another connection or relaxes the production guard or assertions.
That modified target passed. This is not proof of a full green Core suite.

The exact 45-to-46 procedure is core-channel-observations-v1. Actual PostgreSQL
catalog comparison found 61 added definitions and no removed definitions; all
match the observation table and private upgrade receipt. Earlier 44 and 45
upgrade targets remain frozen. Independent fresh installation completed the
entire CLI with exit0; actual restricted runtime check-db, create/grant/revoke/
regrant and excessive-authority refusal passed. Both new tables stayed empty.
The earlier unpinned capture's committed construction followed by exit1 remains
a failed initialization, not a passing installation.

- Component46 manifest: 7e69eafaa516a67dfc1b719ae2f5a950a90596f912f47d187422c470f9c170ee
- Component46 catalog: c1b305360f39946be511368c8aba21635158dc767ca65eaab380784673a5d9a6
- Independently installed Core: 79932aae8473e4c74a9833e1369ae7b625020f0e1542aac89f8e168694a8fad2
- Subsequent Chat V2 Core build: 54973d793e10b3b100cf71af87f515b193ccc0ed43c8fc96782516dbbc71b6ec
- Chat V2 test executable: 7fc35094e84402f95436f141ab298d2ed932cfd7e16a098178c180155aa36911

The two executable cohorts are distinct. Their compiler inputs and scoped
results are retained under ~/.cache/lmm-module-migration/evidence in core46-frozen,
core46-targets, observations46-acceptance and audio-v31-tests.log. Private
observation fixtures were stopped and removed; public captures and failures
remain. Audio tests use synthetic providers and gateway reconstruction, not
paid providers or operating-system crash/restart acceptance. The older v3
journal fixture, remaining audio entries, final projection/resource tests,
sorting browser acceptance, imports and production cutover remain unfinished.

## Earlier projection and trust-policy integration

The earlier delivery extended source base 6ae84b4e5ab65120c5542ae9d2ed2921a9ba2da8
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

## Earlier local evidence

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
| Ignored payment gateway PostgreSQL follow-up | Passed after correcting five aggregate test expectations to include the already-configured Coinbase profile; all six profiles and balanced journals checked |
| Ignored email PostgreSQL follow-up | All 17 cases passed from the current complete Core test executable, using a private PostgreSQL cluster and synthetic SMTP peers |

The original Core failures and their follow-ups are separate evidence; the
complete library suite was not rerun after those targeted fixes. The separately
run email and payment cases do not turn that historical aggregate into a green
run; the ignored routing benchmark remains unrun. The trust service's
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

## Independent trust-policy browser acceptance

A different owner verified 27 cases against source
81d2c65f86dc9637b5fb4d011f655196dc8e8be2, the Core executable above, a fresh
private 45-component database and the actual bundled Web application. Actual
login, offline enrollment and fingerprint approval, user-verified WebAuthn
confirmation, persistent audit, reload, Core restart and editor revocation
passed. Ordinary administrators, API keys and a runtime-forged root session
with an unapproved security key could not change the policy. Wrong signed
WebAuthn origin, stale preview, public-only export, cancellation and the
390-pixel read-only page were also checked.

The real Go host was built and its health endpoint checked, with modules set to
none; trust-policy requests are owned by Core. Its executable SHA-256 is
48c45ce698bf3e92165a0df20a2f1d04bd90e219d01a47ca47fed652e885ce95.
This does not cover Go extension business or Core-Go RPC. The browser used a
CTAP2 virtual authenticator with user verification. Physical keys,
existing-site upgrades and the full-site route/theme matrix remain unrun in
this acceptance.

A further 15 independent checks used the same frozen Core, Go and Web artifacts.
Actual TOTP setup, invalid-code rejection, old-session revocation and the real
OTP login screen passed; the policy audit was bound to the resulting persisted
MFA session. Preview and challenge expiration used the actual five-minute wait,
without changing clocks or database expiration values. A fresh authorized
preview subsequently committed successfully.

The other cases suppressed the real PostgreSQL COMMIT acknowledgment and,
separately, disconnected the successful HTTP confirmation response. The browser
kept the original review, did not repeat confirmation, and resolved the original
operation once. Database observations before and after resolution proved one
policy revision and audit per operation. These supplemental cases and their
retained harness failures are recorded under
core-go-trust-browser/gaps-mfa-ttl-commit. They do not expand Go business coverage.

Public results, artifact bindings, screenshots and runners are retained locally
under ~/.cache/lmm-module-migration/evidence/core-go-trust-browser. Its private
fixture processes were stopped and credentials removed. The payment and email
follow-ups are recorded separately in gateway-proof-corrected.log and
current-email-ignored; the earlier payment aggregate failure is retained.
Only test expectations changed for that payment fix, not production logic.

## Still incomplete

Other audio entry points and process-level recovery, the full provider/format
matrix, sorting browser and final freshness/resource acceptance, final identity
import-contract review, complete financial capability parity, and final resource/rollback
acceptance remain open. The importer candidate is intentionally held for the
final schema review; source-fingerprint rejection still blocks its dependent
trust-import suite. No fingerprint was changed merely to make that suite pass.

Beyond the trust-policy slice above, real Core/Go browser business acceptance,
final million-record projection evidence, independent whole-business sign-off
and production deployment are not complete. No production user, account,
business database or running service was changed by these local checks.
