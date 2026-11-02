# Trust editor integration contract

Single schema component name: `trust-policy-edit`. Concatenate, in this order:
`services/core/schema/trust_policy_edit.sql`, `trust_policy_edit_proof.sql`,
`trust_policy_edit_editor.sql`. These are one unpublished atomic component, not
three independently enabled protocols. No preseeded signing key or editor.

The companion `options_http.patch` adds an opt-in
`OptionsHttp::with_trust_policy_editor(service)` builder and mounts all five routes.
It is not applied by this owner. Keep the option disabled until frontend wiring and
central acceptance are complete.

Core public API (export `pub mod trust_policy` from `options_http.rs`):

```rust
EditorConfig::new(rp_id: &str, origin: &str) -> Result<EditorConfig, Error>
SigningKey::new(bytes: [u8; 32]) -> SigningKey
Service::new(pool: PgPool, config: EditorConfig, key: SigningKey) -> Service
Service::check_ready(&self) -> Result<(), Error>
Service::read(&self, session: &str) -> Result<PolicyRead, Error>
Service::preview(&self, session: &str, value: &str) -> Result<CreatedPreview, Error>
Service::challenge(&self, session: &str, token: &str, digest: &str, editor: &str) -> Result<Challenge, Error>
Service::confirm(&self, session: &str, input: &Confirmation) -> Result<i64, Error>
Service::resolve(&self, session: &str, token: &str) -> Result<Resolution, Error>
http::router(service: Service) -> axum::Router
editor::Owner::new(owner_pool: PgPool, config: EditorConfig) -> Owner
Owner::begin_registration(&self, editor: &str, actor: i64, expected_version: i64) -> Result<Registration, Error>
Owner::review_registration(&self, enrollment: &str, response: &RegisterPublicKeyCredential) -> Result<Review, Error>
Owner::approve_registration(&self, enrollment: &str, fingerprint: &str) -> Result<i64, Error>
Owner::revoke(&self, editor: &str, expected_version: i64) -> Result<i64, Error>
Owner::registration_status(&self, enrollment: &str) -> Result<RegistrationStatus, Error>
Owner::editor_status(&self, editor: &str) -> Result<Option<EditorStatus>, Error>
```

Config additions owned by integration owner:
`trust_policy_editor.enabled: bool` (default false), `rp_id: String`,
`origin: String`, `signing_key_file: PathBuf` (exactly 32 random bytes, no fallback,
no reuse of MFA/session keys). Instantiate only with all fields valid and the
component, runtime ACL and matching DB signing key provisioned. Secret file
permissions/ownership must be checked by the shared loader; never log its bytes.
The `SigningKey` type has no Debug/Serialize/Default and zeroizes on final drop.
The initial array remains the loader's responsibility to zeroize after transfer.
No database-owner URL belongs in the online service configuration.

Owner key provisioning: install the same random bytes into the owner-only
`core_trust_pricing.policy_edit_signing_key(secret)` out of band. Do not place
literal secrets in scripts, history, SQL text, migration snapshots or error
reports. Provision using the existing protected owner secret channel, with SQL
parameter logging (including error/audit paths) disabled and verified. Failure to
provision or a key mismatch must fail closed; no fixture key is selected automatically. Signing-key rotation is separate from passkey rotation: coordinate
owner row and process secret activation; mismatched processes fail closed.

Exact runtime grants (replace `runtime_role` using the existing safe role binder):

```sql
GRANT EXECUTE ON FUNCTION
 core_trust_pricing.read_policy_edit_description(bigint),
 core_trust_pricing.read_policy_editors(bigint),
 core_trust_pricing.policy_editor_ready(bytea),
 core_trust_pricing.policy_edit_binding(text,jsonb),
 core_trust_pricing.authorized_policy_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea,bytea),
 core_trust_pricing.authorized_policy_resolve(bytea,bigint,bigint,bytea),
 core_trust_pricing.policy_editor_context(text,bigint),
 core_trust_pricing.authorized_policy_challenge(jsonb,bytea),
 core_trust_pricing.policy_challenge_context(bytea,bytea,bigint,bigint),
 core_trust_pricing.authorized_policy_confirm(jsonb,bytea)
TO runtime_role;
```

Retain existing policy SELECT and schema USAGE. No runtime table grants on any
`policy_edit*` table, no key/HMAC-verifier grant, no legacy create/apply/resolve
primitive grant, no owner-role membership. Remove the obsolete five-argument
`authorized_policy_confirm(bytea,bigint,bigint,bytea,bytea)` from inventories:
it no longer exists. Runtime checks must reject extra write/execute privileges,
not just verify that expected grants exist. Owner commands enforce exact table
ownership in addition to the database ACL. They have no HTTP routes.

Offline ceremony: operator independently verifies actor ID and editor identity,
starts registration over the owner connection, transports *public options* to a
trusted browser at the exact configured HTTPS origin, returns its WebAuthn
registration response to `review_registration`, independently checks the returned
actor/editor/RP/origin/credential fingerprint, then explicitly calls
`approve_registration` with that fingerprint. Expected version 0 is initial
registration; a positive current version is atomic replacement. Revoke increments
the version; replacement after revoke uses that incremented version. There is no
ordinary root enrollment, recovery code, password reset or runtime override.
Owner access is the recovery path if all editor devices are lost.
After an uncertain review/approval outcome, use `registration_status`; after an
uncertain revoke, use `editor_status`. Do not blindly repeat mutations. Reviewed
status returns the same fingerprint; approved status retains its original version
even if the editor has since been rotated.

Web flow: show preview current/proposed values and financial effects; keep token
and digest intact. On explicit Confirm, POST challenge with token/digest/editor,
call the existing WebAuthn browser converter and `navigator.credentials.get`, then
POST confirm with token/digest/challenge/assertion. Never mark an unauthenticated
checkbox as passkey verification. Missing assertion fails JSON extraction. Retain existing BFF/CSRF/session protections. No
generic `/api/option` PUT fallback or broader writable-key whitelist. Cancellation
makes no policy change. For lost confirm response call resolve with the original
token; `confirmed` is final, `pending` is not success, and `unknown`/`expired` require
a fresh reviewed preview. Each preview permits one challenge; if its response is
lost, create/review a new preview (old preview expires within five minutes).

Security boundary: possession of the runtime DB login may forge root identity and
MFA rows and induce previews. It cannot rewrite independent editor credentials or
satisfy the assertion verifier. This does not claim safety against Core process/
HMAC-key compromise, owner compromise or malicious browser code. A passkey does
not display or independently attest the policy's financial contents to its user.

Atomicity: identity locks precede policy advisory lock 741803219. Owner operations
and confirms take its exclusive mode; challenge insertion uses shared mode.
Confirmation rechecks editor version, revocation, preview/session binding, expiry,
latest authenticator counter, and policy revision. Counter update, audit, policy
write and challenge consumption commit together. Only verified counter and backup
flags are updated, never credential ID/public key/RP/origin/version. A concurrent
owner revoke either follows an already committed confirm or fences it out. Failed
verification/CAS leaves policy, counter and consumption unchanged. Resolve may
recover a committed outcome from a renewed session for the same actor.

No Cargo/DB execution by this owner. Central owner should run on isolated synthetic
DB only: `cargo test -p lmm-core --lib options_http::trust_policy -- --test-threads=1`
and independently verify fresh/upgrade catalogs, exact ACLs, missing/wrong key,
restart/recovery, owner CLI ceremony at real browser origin, and existing native
passkey/browser security regressions. No production data or real paid calls.

Static handoff evidence (this owner): original 12 test names retained, 11 new
security tests added; rustfmt parsed all owned Rust modules successfully; all
new SQL functions have explicit PUBLIC revokes, trigger targets exist, the old
five-argument confirm overload is absent, and production modules contain no
fixture signing key. `git apply --check` accepts `options_http.patch`. These are
static checks only: no compilation, PostgreSQL execution or browser acceptance
was performed. The local SQL parser package is unavailable; central execution
must validate PL/pgSQL and actual grants.

Formal initialization: `GET /core/v1/admin/trust-policy` uses the same root-session
bearer authorization and no-store response helper as the editing routes. It is
mounted only with the explicitly configured Service (shared integration must keep
all trust routes absent unless enabled and configuration/schema/key checks pass).
The response is `{success:true,data:{revision,policy,console,editors}}`. `policy`
is validated numeric TrustPolicy JSON returned unchanged from storage (rational
values are not normalized); `console` is the verified description or null when
history lacks it. `editors` contains only this actor's owner-enrolled entries,
ordered by editor_id, with `{editor_id,version,status}`. Status is `active`,
`revoked`, or `configuration_mismatch` (stored RP/origin differs from this
Service). It never returns public keys, credential IDs, enrollment state or HMAC
material. An empty list means the UI must explain that offline owner registration
is required; no default editor or policy is synthesized. Only active entries may
be selected; confirmation still rechecks everything after this read.
Additional exact runtime GRANT: `core_trust_pricing.read_policy_editors(bigint)`.
The central owner's metadata-only `trust_policy_upgrade.sql` is a fourth input
to this same component; this owner has not modified it or frozen component 44.

Read endpoint handoff: implemented and ready to freeze. The added isolated test
covers restricted-runtime reads, exact raw 94/100 retention, null historical
description, other-actor empty list, root/admin/missing-session HTTP authorization,
no-store, active/configuration-mismatch/revoked status, and post-confirm description
readback. Rust formatting/syntax parsing passed; Cargo/PG remain unrun here.

Startup readiness (required before publishing any enabled route): call
`service.check_ready().await?` after schema/runtime ACL validation and secret
injection, before passing the Service to the enabled router. It opens one explicit
READ ONLY transaction, signs action `ready` with exactly `{}`, then calls
`core_trust_pricing.policy_editor_ready(bytea)` in that same transaction. This
SECURITY DEFINER function only calls `policy_edit_verify('ready','{}',proof)`;
there is no generic verifier grant, user/session dependency or application write.
False (including missing DB key or mismatched injected key) becomes Error::Storage;
SQL/connection/ACL errors also propagate and must abort enabled startup. Do not
catch them and silently publish the enabled routes. This is a startup observation,
not a promise against later owner key rotation; each mutation still verifies its
own proof. Add the exact grant `core_trust_pricing.policy_editor_ready(bytea)`.
The added isolated test covers matching/wrong/missing key with zero identity users,
unchanged application state, action/payload separation, malformed proof,
transaction replay rejection and denial of direct generic verifier access.
Static Rust syntax/format checks only; Cargo and PostgreSQL execution remain with
the central owner. Readiness implementation is complete and ready to freeze.
