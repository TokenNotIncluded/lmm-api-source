# Federated authentication configuration

Federated authentication is a Core login client capability. It does not enable
the site's OAuth authorization server, remote clients or API-key authority.
The host capability defaults to disabled. Its database namespace is exclusively
`core_federated_auth`, installed by the appended `federated-auth` component.
Historical identity SQL is unchanged.

Install or explicitly upgrade the fixed installation target with the owner
workflow before starting a new runtime. The serving process performs checks;
it does not create schema components. Grant runtime privileges with the
deployment script's `enable_federated_auth=true` flag. The flag grants ACLs
only: it neither enables HTTP authentication nor configures any provider.
Use the explicit optional-ACL reset when removing privileges, then check-db
with the desired required capabilities. Runtime receives no namespace CREATE,
provider/identity/receipt DELETE or TRUNCATE, or function EXECUTE privilege.

The [reference configuration](../deployment/docker/config/core-federated-auth.example.toml)
shows `[identity.federated_auth] enabled=true` and `keyring_file`. Enabling
requires a database and `[identity.web]`. Missing keys, inconsistent disabled
configuration containing a keyring, missing installed schema/ACLs, and an
unreadable provider secret prevent startup. They do not silently disable login.
The legacy environment equivalents are `LMM_CORE_FEDERATED_AUTH_ENABLED` and
`LMM_CORE_FEDERATED_AUTH_KEYRING_FILE`; supplying the keyring without an explicit
flag enables validation. Explicit false plus a keyring is an error.

The private keyring is TOML or JSON, version 1. For example, the **private**
document at the configured path has this shape; no key material belongs in Git:

```toml
version = 1
active_key_id = "first"
replay_key_file = "/run/secrets/core_federated_replay_key"

[[keys]]
id = "first"
key_file = "/run/secrets/core_federated_encryption_key"
```

Every key file contains exactly 32 raw random bytes, not their hexadecimal or
base64 representation. Paths must be absolute, regular, private files without
symlink traversal. Restrict them to the service owner, normally mode 0600.
Keep these keys separate from Web refresh, MFA, provider client secrets and the
WeChat bridge token. At most eight encryption keys are retained. Provider
secrets use authenticated encryption bound to an immutable random provider
context, revision and key label. Read APIs return an empty secret field and a
configured indicator; an empty secret on update preserves the existing value.

Keep old encryption keys until all retained nondeleted providers have been
resealed under the active key. The replay key is independent and remains stable
when encryption keys rotate or retire. Replacing it requires an explicit
offline period sufficient for issued bridge codes and the ten-minute durable
claims to expire. Never use active encryption-key rotation to reset replay
protection. Neither keys nor plaintext provider secrets are persisted in
public configuration, provider responses, URLs or operation receipts.

Provider enable/disable is separate from the host capability. Disabling a
provider blocks new login/binding and invalidates outstanding flows through its
monotonic configuration revision. It does not silently revoke every existing
Core session: an issued session retains the normal Core authority rules.
Binding and unlinking rotate the user's auth version and revoke other sessions
and credentials through the existing native security-rotation path. An unlink
must leave another configured primary login method. Provider deletion retains
its identity namespace and is rejected while active bindings remain.

WeChat uses the configured public verification-code bridge's
`/api/wechat/user?code=…` endpoint with its raw Authorization token. It is not a
replacement QR-code OAuth provider. Browser cookie binding, durable one-time
flow consumption and provider-scoped HMAC claims separate login from account
binding; provider email, display names and roles never choose an existing Core
account or grant permissions.
