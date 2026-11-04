# Referral rewards to wallet

Core serves the browser confirmation independently of Go. The native wallet
links to `GET /core/v1/affiliate/claim`; an optional `amount_credits` query
parameter chooses an integer amount, otherwise Core previews the current
positive affiliate balance. The minimum is 500000 CREDIT. Negative rewards
are not converted into wallet debt or erased.

The Core page explicitly confirms through
`POST /core/v1/affiliate/claim/{id}` with its private `csrf` value. Confirmation
requires the original user's live refresh-cookie session and the configured
web Origin. A bearer token alone does not authorize it; neither the amount
nor a destination account is supplied by the confirmation request.

Login and refresh issue matching HttpOnly, SameSite=Strict cookies scoped
to `/api/user/auth` and `/core`; rotation and logout replace or clear both.
The Core cookie covers native confirmation routes without sending the refresh
proof to Go routes or the rest of `/api`. Existing sessions acquire that
cookie through normal refresh or a new login.

The operation moves existing entitlement through the controlled ledger
`ClaimAffiliate` action. It creates no new referral reward. Wallet debt is
repaid first. Preview consumption and the posted or rejected ledger outcome
commit together. Repeating the same confirmation returns its original
outcome; a rejected preview does not become successful after later funding.
A failed HTTP response, invalid JSON or lost response is an unknown result;
the page retains the same preview and allows its original confirmation to
be retried rather than asking the user to create another transfer.
New previews expire after ten minutes. A terminal receipt remains readable
by the original live session after expiry or payment-policy withdrawal.

New previews and unconsumed confirmations require the existing operator
payment policy: `payment_setting.compliance_confirmed=true` and
`payment_setting.compliance_terms_version=v1` in `core_console.options`.
The `console` phase in [the cutover importer](../scripts/legacy_cutover_keys.py)
copies those facts exactly. Missing consent, a false flag or an older version
does not authorize claiming; migration must not invent consent.

Fresh Core schema component 52, `affiliate-claims`, stores session-bound
previews and durable outcomes. [The runtime grant script](../deployment/postgres/grant-core-runtime.sql)
adds its minimal permissions when web identity is enabled. This component
does not supply an existing-database upgrade from component 51; ordinary
startup still performs no installation or migration.

This capability does not settle the imported referral queue, generate its
pending tail rewards, convert discount facts into native coupons, or prove
the full historical-wallet and page acceptance scope complete.
