# Magpie backend

The managed OAuth client is `lmm-magpie`, bound to application `magpie`.
It is disabled by default. Enable its finite ceiling separately with
`identity.oauth2.magpie_max_credits` (1 through 9007199254740991); model
configuration and a managed-refresh keyring are also required. CC Switch, Pi
and browser authorization do not enable Magpie or supply its credentials.

Consent binds the selected allowance and model groups to one installation and
family. Access-token generations and refresh recovery share that family's
allowance. Retrying the same rotation recovers the same successor within the
existing 600-second window; replaying a consumed refresh with another rotation
retains the existing family-revocation policy.

`GET /api/oauth2/application/budget` accepts a current managed Bearer with
`usage:read`, without query parameters. Its `lmm.application-budget.v1` response
identifies the client, application, installation and grant, with integer
`budget_credits`, `consumed_credits`, `reserved_credits` and `remaining_credits`
in `credit_500k_usd`. Consumption includes the family's entire lifetime and
settled refunds; pending and reconciliation reservations remain occupied.
Permission and accounting reads use one repeatable-read transaction. The
endpoint does not call a provider, rotate credentials or alter funds.

The unpublished managed-applications component 70 includes the client/application
pair constraints. Its exact 69-to-70 owner upgrade must use the newly captured
target catalog; the deployed component-69 predecessor remains unchanged.
Later schema components are separate transitions.

Backend self-tests do not establish native Magpie host, real TLS, durable host
journal, installed-plugin or paid-model acceptance. Those checks remain separate.
