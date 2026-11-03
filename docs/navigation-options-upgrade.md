# Public navigation persistence

Core accepts `HeaderNavModules` through the existing superadministrator-session
settings endpoints. The settings response advertises that key; the restricted
Web settings page mounts the existing navigation editor only when that live
capability is present. API keys, ordinary users and non-root administrators
cannot save it. Other unsupported options remain read-only.

The value is a JSON object limited to 64 KiB. Known switches accept the existing
boolean, 0/1 and boolean-string representations; pricing, rankings and security
also accept enabled/requireAuth objects. Unknown extension fields are retained.
Writes preserve the existing transaction lock, optional bulk expected-value
comparison, revisions, audit and uncertain-commit response. A failed audit rolls
back the entire batch. Defaults are represented as `{}` when no row exists.

`/api/status` reads the stored navigation configuration when Core console is
enabled. A failed read returns 503 with no-store instead of presenting default
navigation as current configuration. The public `server_address` and the
read-only `ServerAddress` settings entry use the effective `LMM_CORE_WEB_ORIGIN`.
This does not change OAuth issuer discovery or permit editing host configuration.

## Installation and upgrade

Component 47, `navigation-options`, adds the private
`core_console.write_navigation_option(text,text)` function and an owner-only
immutable upgrade receipt. The original 46 component SQL definitions and the
existing public-option writer remain unchanged. The new writer accepts only
HeaderNavModules; it does not grant direct option-table writes.

Fresh installation includes component 47. Existing component-46 installations
use `core-navigation-options-v1` through the existing offline `upgrade-db`
interface. The exact predecessor, successor and database/role identities are
bound in the request; the transaction preserves prior rows, sequences, function
definitions and privileges. An exact replay resolves the original receipt.
The captured successor is fixed in
[the target contract](../services/core/src/core_schema/upgrade/navigation_options_target.json).
Older upgrade procedures retain their separately frozen successors.

After the offline schema upgrade, the owner must apply the console section of
[the runtime grants](../deployment/postgres/grant-core-runtime.sql) to grant
EXECUTE on the new function. Runtime startup checks this permission and function
ownership. Ordinary startup never upgrades schema or grants privileges.

## Verification

Local PostgreSQL 18.6 verification passed 31 settings/editor tests, four upgrade
cases (including the unchanged 45→46 path and the new 46→47 path), and the
restricted-console real TCP scenario. The latter uses the official grants and
checks concurrent navigation writes, permission refusals and audit-failure
rollback. The Web passed 14 focused settings/capability tests, typecheck, scoped
lint/format, production build and bundle budget (559346 / 786432 gzip bytes).

The first upgrade candidate replaced the old writer and failed the frozen-function
assertion; the final candidate adds a separate writer and passed unchanged
preservation assertions. The initial settings run passed 25 and failed six at
the fixture's localhost URL assertion; the corrected private connection URL
passed all 31. The new UI test initially selected a nonexistent generic submit
button; using the actual Save navigation button passed. Original failed logs
remain under `~/.cache/lmm-navigation-validation/`.

Commands are run from the source repository with a private PostgreSQL test URL:

```sh
cargo test --manifest-path services/core/Cargo.toml --locked --lib options_http::
cargo test --manifest-path services/core/Cargo.toml --locked --test navigation_options_upgrade --test channel_observations_upgrade -- --test-threads=1
bash scripts/test-console-runtime.sh
```

This is local developer verification, not independent business acceptance. Actual
browser login/relogin, Core process restart, desktop and 390/320-pixel navigation
acceptance remain open. No production data or servers are used.
