---
type: architecture
title: Browser identity and account lifecycle
description: Explains Core browser registration, login and refresh sessions, MFA and account security, and the guarded account closure workflow.
tags: [identity, browser, sessions, mfa, account-lifecycle]
verified:
  - by: openwiki/0.7.2
    at: 2026-11-01T04:43:29.750Z
sources:
  - id: openwiki-source-0ab042fa99f84cec9f84ae90
    resource: repo://services/core/src/identity/browser.rs
  - id: openwiki-source-568383381c3ed8f1d169e6c7
    resource: repo://services/core/src/identity/browser/account_lifecycle.rs
  - id: openwiki-source-71437e253a78961e1f0bcc04
    resource: repo://services/core/tests/account_closure.rs
generated: { by: "codex", at: "2026-11-01T04:43:29.750Z" }
---

# Browser identity and account lifecycle

## Core browser authority

Browser identity is implemented by `BrowserAuth` over the Core `IdentityStore`. It validates password-registration policy, performs login and MFA challenge flows, and issues browser session bundles. Refresh secrets are derived with a server-held key and bound to the session family; refresh runs transactionally and handles concurrent presentation of a previous token within the defined race window. Unknown secrets do not revoke otherwise valid sessions.

Registration currently rejects non-empty email because email verification is not implemented; the handler does not silently discard the requested address. MFA-required login does not return a normal session until the challenge is completed. See [Core identity ownership](core-identity-and-data.md).

## Session and security operations

The browser identity modules include session refresh/logout, MFA, passkeys, external login, account links, account appeals, and profile/account lifecycle operations. Each operation remains within Core identity and its permission checks. Web components call these APIs; UI controls do not grant the underlying authority. See [Web and clients](../integrations/web-and-clients.md).

## Account closure

Account closure is a preview-then-apply workflow. The preview evaluates active entitlements and in-flight state such as team ownership, model reservations, active subscriptions, pending payment/refund records, paid commerce orders, and pending transfers. Closure requires an operation key, explicit confirmation, and the expected auth version; it runs in a transaction and rejects if the account changed or has blockers. The closure flow preserves history while revoking authority rather than deleting financial records.

## Focused tests

`services/core/tests/identity_http.rs` exercises native HTTP identity behavior and verifies database failure does not grant access. `identity_mfa.rs` checks bound challenges, attempt accounting, and refresh/enrollment interactions. `account_closure.rs` covers idempotent closure, blockers, preserved records, and concurrent credit/closure behavior with PostgreSQL fixtures. These are service and database tests; they do not substitute for browser acceptance of the current Web routes.
